//! `cxpak schema` — print the JSON Schema for a capability's output contract.
//!
//! Each capability schema is versioned independently (ADR-0169, extended in Task 0.5):
//! consumers can pin to a `format_version` and detect breaking changes. Schemas
//! document the existing JSON output rather than introducing new serialization formats.
//!
//! Capabilities: `context` (= auto_context, "2.3"), `graph` ("3.0"), `data` ("3.0"),
//! `review` ("3.0").

use serde_json::json;

// ---------------------------------------------------------------------------
// Per-capability format versions
// ---------------------------------------------------------------------------

/// Format version for the `context` capability (= auto_context output contract).
/// Must remain in sync with [`crate::auto_context::FORMAT_VERSION`].
/// Separated here so the schema const can reference it without bumping the
/// auto_context FORMAT_VERSION (Task 0.5 does not change auto_context output).
const CONTEXT_FORMAT_VERSION: &str = crate::auto_context::FORMAT_VERSION;

/// Format version for new capabilities introduced in cxpak 3.0 (graph/data/review).
const CXPAK_30_FORMAT_VERSION: &str = "3.0";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Returned by [`capability_schema`] when the requested id is not registered.
#[derive(Debug)]
pub struct UnknownCapabilityError(pub String);

impl std::fmt::Display for UnknownCapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "unknown capability '{}'; known ids: context, graph, data, review",
            self.0
        )
    }
}

impl std::error::Error for UnknownCapabilityError {}

// ---------------------------------------------------------------------------
// Registry + dispatch
// ---------------------------------------------------------------------------

/// Return the versioned JSON Schema for the named capability, or an error if
/// the id is not registered.
///
/// Known ids: `context`, `graph`, `data`, `review`.
pub fn capability_schema(id: &str) -> Result<serde_json::Value, UnknownCapabilityError> {
    match id {
        "context" | "auto_context" => Ok(auto_context_schema()),
        "graph" => Ok(graph_schema()),
        "data" => Ok(data_schema()),
        "review" => Ok(review_schema()),
        other => Err(UnknownCapabilityError(other.to_string())),
    }
}

// ---------------------------------------------------------------------------
// context / auto_context schema (unchanged — "2.3")
// ---------------------------------------------------------------------------

/// Build the JSON Schema document for `AutoContextResult`, keyed to the current
/// `FORMAT_VERSION`. Kept in one place so the advertised contract and the const
/// never drift.
pub fn auto_context_schema() -> serde_json::Value {
    let version = CONTEXT_FORMAT_VERSION;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "AutoContextResult",
        "description": "Token-budgeted context bundle produced by cxpak auto_context.",
        "type": "object",
        "x-format-version": version,
        "required": [
            "format_version", "task", "budget", "sections", "filtered_out", "efficiency"
        ],
        "properties": {
            "format_version": {
                "type": "string",
                "const": version,
                "description": "Version of this output contract; bumped only on breaking changes."
            },
            "task": { "type": "string" },
            "dna": { "type": "string", "description": "Convention-profile DNA section." },
            "budget": {
                "type": "object",
                "required": ["total", "used", "remaining"],
                "properties": {
                    "total": { "type": "integer", "minimum": 0 },
                    "used": { "type": "integer", "minimum": 0 },
                    "remaining": { "type": "integer", "minimum": 0 }
                }
            },
            "sections": {
                "type": "object",
                "description": "Packed context sections.",
                "required": [
                    "target_files", "test_files", "schema_context",
                    "api_surface", "blast_radius", "cross_language_tokens"
                ],
                "properties": {
                    "target_files": { "$ref": "#/$defs/packedFileSection" },
                    "test_files": { "$ref": "#/$defs/packedFileSection" },
                    "schema_context": { "$ref": "#/$defs/packedFileSection" },
                    "api_surface": {
                        "type": ["object", "array", "null"],
                        "description": "Public API surface (present-but-null when absent)."
                    },
                    "blast_radius": {
                        "type": ["object", "array", "null"],
                        "description": "Blast-radius analysis of the top files (present-but-null when absent)."
                    },
                    "cross_language_edges": {
                        "type": ["object", "array", "null"],
                        "description": "Cross-language boundary edges. OMITTED from the JSON when absent (serde skip_serializing_if), unlike api_surface/blast_radius which serialize as null."
                    },
                    "cross_language_tokens": {
                        "type": "integer", "minimum": 0,
                        "description": "Token count consumed by the cross-language edges section."
                    }
                }
            },
            "filtered_out": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["path", "reason", "tokens"],
                    "properties": {
                        "path": { "type": "string" },
                        "reason": { "type": "string" },
                        "tokens": { "type": "integer", "minimum": 0 }
                    }
                }
            },
            "efficiency": { "$ref": "#/$defs/efficiencyReport" },
            "health": { "type": "object", "description": "Index health score." },
            "risks": { "type": "array", "description": "Top risk entries." },
            "architecture": { "type": "object", "description": "Architecture map." },
            "co_changes": { "type": "array", "description": "Mined co-change edges." },
            "recent_changes": { "type": "array" },
            "predictions": {
                "type": ["object", "null"],
                "description": "Change-impact predictions when the task mentions file paths."
            }
        },
        "$defs": {
            "packedFileSection": {
                "type": "object",
                "required": ["count", "tokens", "files"],
                "properties": {
                    "count": { "type": "integer", "minimum": 0 },
                    "tokens": { "type": "integer", "minimum": 0 },
                    "files": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["path", "score", "tokens"],
                            "properties": {
                                "path": { "type": "string" },
                                "score": { "type": "number" },
                                "detail_level": { "type": "string" },
                                "tokens": { "type": "integer", "minimum": 0 },
                                "content": { "type": ["string", "null"] }
                            }
                        }
                    }
                }
            },
            "efficiencyReport": {
                "type": "object",
                "description": "Decision-support efficiency report (ADR-0168).",
                "required": [
                    "repo_tokens", "selected_tokens", "relevant_coverage",
                    "relevant_total", "relevant_covered", "absolute_coverage",
                    "budget_total", "budget_used", "budget_utilization",
                    "tokens_saved_filtering", "advisory"
                ],
                "properties": {
                    "repo_tokens": { "type": "integer", "minimum": 0 },
                    "selected_tokens": { "type": "integer", "minimum": 0 },
                    "relevant_coverage": {
                        "type": "number", "minimum": 0, "maximum": 1,
                        "description": "Headline: relevant candidates packed / relevant candidates."
                    },
                    "relevant_total": { "type": "integer", "minimum": 0 },
                    "relevant_covered": { "type": "integer", "minimum": 0 },
                    "absolute_coverage": {
                        "type": "number", "minimum": 0, "maximum": 1,
                        "description": "Demoted sanity field: selected/repo tokens."
                    },
                    "budget_total": { "type": "integer", "minimum": 0 },
                    "budget_used": { "type": "integer", "minimum": 0 },
                    "budget_utilization": { "type": "number", "minimum": 0, "maximum": 1 },
                    "marginal_included_score": {
                        "type": ["number", "null"],
                        "description": "Lowest relevance score among included files."
                    },
                    "marginal_excluded_score": {
                        "type": ["number", "null"],
                        "description": "Highest relevance score among excluded relevant files."
                    },
                    "tokens_saved_filtering": { "type": "integer", "minimum": 0 },
                    "cost_estimate": {
                        "type": ["object", "null"],
                        "properties": {
                            "model": { "type": "string" },
                            "input_usd": { "type": "number" },
                            "rates_dated": { "type": "string" }
                        }
                    },
                    "advisory": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Derived guidance; empty when the selection is healthy."
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// graph schema — DependencyGraph + TypedEdge + EdgeConfidence (Task 0.4)
// ---------------------------------------------------------------------------

/// JSON Schema for the dependency-graph capability.
///
/// Derived from `src/core_graph/graph.rs`: `DependencyGraph` (edges/reverse_edges
/// keyed by file path), `TypedEdge` (target, edge_type, confidence), and the
/// `EdgeConfidence` enum added in Task 0.4 (Extracted/Inferred).
fn graph_schema() -> serde_json::Value {
    let version = CXPAK_30_FORMAT_VERSION;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "DependencyGraph",
        "description": "Dependency graph produced by cxpak indexing. Nodes are file paths; edges are typed and carry confidence.",
        "type": "object",
        "x-format-version": version,
        "required": ["edges", "reverse_edges"],
        "properties": {
            "edges": {
                "type": "object",
                "description": "Map from source file path to the set of outgoing typed edges.",
                "additionalProperties": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/typedEdge" }
                }
            },
            "reverse_edges": {
                "type": "object",
                "description": "Map from target file path to the set of incoming typed edges (reverse direction).",
                "additionalProperties": {
                    "type": "array",
                    "items": { "$ref": "#/$defs/typedEdge" }
                }
            }
        },
        "$defs": {
            "typedEdge": {
                "type": "object",
                "required": ["target", "edge_type", "confidence"],
                "properties": {
                    "target": {
                        "type": "string",
                        "description": "Target file path."
                    },
                    "edge_type": {
                        "description": "Semantic type of this dependency edge.",
                        "oneOf": [
                            { "type": "string", "enum": [
                                "Import", "ForeignKey", "ViewReference", "TriggerTarget",
                                "IndexTarget", "FunctionReference", "EmbeddedSql",
                                "OrmModel", "MigrationSequence"
                            ]},
                            {
                                "type": "object",
                                "description": "CrossLanguage edge with a BridgeType payload.",
                                "required": ["CrossLanguage"],
                                "properties": {
                                    "CrossLanguage": {
                                        "type": "string",
                                        "enum": [
                                            "HttpCall", "FfiBinding", "GrpcCall",
                                            "GraphqlCall", "SharedSchema", "CommandExec"
                                        ]
                                    }
                                }
                            }
                        ]
                    },
                    "confidence": {
                        "type": "string",
                        "enum": ["Extracted", "Inferred"],
                        "description": "Whether this edge was structurally Extracted (explicit source) or heuristically Inferred (pattern matching). Added in Task 0.4 / ADR-0176."
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// data schema — SchemaIndex (tables/views/functions/orm_models/migrations)
// ---------------------------------------------------------------------------

/// JSON Schema for the data-layer capability.
///
/// Derived from `src/core_graph/schema.rs`: `SchemaIndex` and all sub-structs
/// (TableSchema, ColumnSchema, ForeignKeyRef, IndexSchema, ViewSchema,
/// DbFunctionSchema, OrmModelSchema, OrmFieldSchema, MigrationChain,
/// MigrationEntry, and their enum types OrmFramework/MigrationFramework).
fn data_schema() -> serde_json::Value {
    let version = CXPAK_30_FORMAT_VERSION;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "SchemaIndex",
        "description": "Data-layer index produced by cxpak: tables, views, DB functions, ORM models, and migration chains.",
        "type": "object",
        "x-format-version": version,
        "required": ["tables", "views", "functions", "orm_models", "migrations"],
        "properties": {
            "tables": {
                "type": "object",
                "description": "Map from table name to TableSchema.",
                "additionalProperties": { "$ref": "#/$defs/tableSchema" }
            },
            "views": {
                "type": "object",
                "description": "Map from view name to ViewSchema.",
                "additionalProperties": { "$ref": "#/$defs/viewSchema" }
            },
            "functions": {
                "type": "object",
                "description": "Map from function name to DbFunctionSchema.",
                "additionalProperties": { "$ref": "#/$defs/dbFunctionSchema" }
            },
            "orm_models": {
                "type": "object",
                "description": "Map from class name to OrmModelSchema.",
                "additionalProperties": { "$ref": "#/$defs/ormModelSchema" }
            },
            "migrations": {
                "type": "array",
                "description": "Ordered migration chains (one per detected framework/directory).",
                "items": { "$ref": "#/$defs/migrationChain" }
            }
        },
        "$defs": {
            "tableSchema": {
                "type": "object",
                "required": ["name", "columns", "file_path", "start_line"],
                "properties": {
                    "name": { "type": "string" },
                    "columns": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/columnSchema" }
                    },
                    "primary_key": {
                        "type": ["array", "null"],
                        "items": { "type": "string" }
                    },
                    "indexes": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/indexSchema" }
                    },
                    "file_path": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 0 }
                }
            },
            "columnSchema": {
                "type": "object",
                "required": ["name", "data_type", "nullable", "constraints"],
                "properties": {
                    "name": { "type": "string" },
                    "data_type": { "type": "string" },
                    "nullable": { "type": "boolean" },
                    "default": { "type": ["string", "null"] },
                    "constraints": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "foreign_key": {
                        "oneOf": [
                            { "$ref": "#/$defs/foreignKeyRef" },
                            { "type": "null" }
                        ]
                    }
                }
            },
            "foreignKeyRef": {
                "type": "object",
                "required": ["target_table", "target_column"],
                "properties": {
                    "target_table": { "type": "string" },
                    "target_column": { "type": "string" }
                }
            },
            "indexSchema": {
                "type": "object",
                "required": ["name", "columns", "unique"],
                "properties": {
                    "name": { "type": "string" },
                    "columns": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "unique": { "type": "boolean" }
                }
            },
            "viewSchema": {
                "type": "object",
                "required": ["name", "source_tables", "file_path"],
                "properties": {
                    "name": { "type": "string" },
                    "source_tables": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "file_path": { "type": "string" }
                }
            },
            "dbFunctionSchema": {
                "type": "object",
                "required": ["name", "referenced_tables", "file_path"],
                "properties": {
                    "name": { "type": "string" },
                    "referenced_tables": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "file_path": { "type": "string" }
                }
            },
            "ormModelSchema": {
                "type": "object",
                "required": ["class_name", "table_name", "framework", "file_path", "fields"],
                "properties": {
                    "class_name": { "type": "string" },
                    "table_name": { "type": "string" },
                    "framework": {
                        "type": "string",
                        "enum": ["Django", "SqlAlchemy", "TypeOrm", "ActiveRecord", "Prisma"]
                    },
                    "file_path": { "type": "string" },
                    "fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/ormFieldSchema" }
                    }
                }
            },
            "ormFieldSchema": {
                "type": "object",
                "required": ["name", "field_type", "is_relation"],
                "properties": {
                    "name": { "type": "string" },
                    "field_type": { "type": "string" },
                    "is_relation": { "type": "boolean" },
                    "related_model": { "type": ["string", "null"] }
                }
            },
            "migrationChain": {
                "type": "object",
                "required": ["framework", "directory", "migrations"],
                "properties": {
                    "framework": {
                        "type": "string",
                        "enum": [
                            "Rails", "Alembic", "Flyway", "Django",
                            "Knex", "Prisma", "Drizzle", "Generic"
                        ]
                    },
                    "directory": { "type": "string" },
                    "migrations": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/migrationEntry" }
                    }
                }
            },
            "migrationEntry": {
                "type": "object",
                "required": ["file_path", "sequence", "name"],
                "properties": {
                    "file_path": { "type": "string" },
                    "sequence": { "type": "string" },
                    "name": { "type": "string" }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// review schema — ContextDelta (auto_context/diff.rs, v2.3.0 W3)
// ---------------------------------------------------------------------------

/// JSON Schema for the review/diff capability.
///
/// Derived from `src/auto_context/diff.rs`: `ContextDelta` (modified_files,
/// new_files, deleted_files, new_symbols, removed_symbols, graph_changes,
/// recommendation) with its sub-types `FileChange`, `SymbolChange`, `GraphChange`.
fn review_schema() -> serde_json::Value {
    let version = CXPAK_30_FORMAT_VERSION;
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "ContextDelta",
        "description": "Review-aware diff delta produced by the cxpak --review capability (v2.3.0 W3).",
        "type": "object",
        "x-format-version": version,
        "required": [
            "modified_files", "new_files", "deleted_files",
            "new_symbols", "removed_symbols", "graph_changes", "recommendation"
        ],
        "properties": {
            "modified_files": {
                "type": "array",
                "items": { "$ref": "#/$defs/fileChange" }
            },
            "new_files": {
                "type": "array",
                "items": { "type": "string" }
            },
            "deleted_files": {
                "type": "array",
                "items": { "type": "string" }
            },
            "new_symbols": {
                "type": "array",
                "items": { "$ref": "#/$defs/symbolChange" }
            },
            "removed_symbols": {
                "type": "array",
                "items": { "$ref": "#/$defs/symbolChange" }
            },
            "graph_changes": {
                "type": "array",
                "items": { "$ref": "#/$defs/graphChange" }
            },
            "recommendation": {
                "type": "string",
                "description": "Derived guidance string for the review context."
            }
        },
        "$defs": {
            "fileChange": {
                "type": "object",
                "required": ["path", "change", "tokens_delta"],
                "properties": {
                    "path": { "type": "string" },
                    "change": {
                        "type": "string",
                        "description": "Human-readable change description."
                    },
                    "tokens_delta": {
                        "type": "integer",
                        "description": "Signed token-count delta (positive = grown, negative = shrunk)."
                    }
                }
            },
            "symbolChange": {
                "type": "object",
                "required": ["path", "name", "kind"],
                "properties": {
                    "path": { "type": "string" },
                    "name": { "type": "string" },
                    "kind": {
                        "type": "string",
                        "description": "Symbol kind string (e.g. Function, Struct, Class)."
                    }
                }
            },
            "graphChange": {
                "type": "object",
                "required": ["change_type", "from", "to", "edge_type"],
                "properties": {
                    "change_type": {
                        "type": "string",
                        "description": "Added or Removed."
                    },
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "edge_type": {
                        "type": "string",
                        "description": "Debug-format EdgeType string."
                    }
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// CLI entry points
// ---------------------------------------------------------------------------

/// Print the schema as pretty JSON to stdout.
///
/// With `capability`: print that capability's schema.
/// Without `capability` (None): preserve back-compat and print the `context`
/// (auto_context) schema — same as the pre-0.5 behaviour.
pub fn run(capability: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let schema = match capability {
        Some(id) => capability_schema(id)?,
        None => auto_context_schema(),
    };
    println!("{}", serde_json::to_string_pretty(&schema)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_advertises_current_format_version() {
        let schema = auto_context_schema();
        assert_eq!(
            schema["properties"]["format_version"]["const"],
            json!(crate::auto_context::FORMAT_VERSION)
        );
        assert_eq!(schema["title"], json!("AutoContextResult"));
        // efficiency block is part of the advertised contract
        assert!(schema["$defs"]["efficiencyReport"].is_object());
        // valid, serializable JSON
        assert!(serde_json::to_string_pretty(&schema).is_ok());
    }

    #[test]
    fn run_prints_without_error() {
        // Exercises the print path; stdout is captured by the test harness.
        assert!(run(None).is_ok());
    }

    #[test]
    fn run_with_capability_id_prints_without_error() {
        assert!(run(Some("context")).is_ok());
        assert!(run(Some("graph")).is_ok());
        assert!(run(Some("data")).is_ok());
        assert!(run(Some("review")).is_ok());
    }

    #[test]
    fn run_unknown_capability_returns_err() {
        assert!(run(Some("bogus")).is_err());
    }
}
