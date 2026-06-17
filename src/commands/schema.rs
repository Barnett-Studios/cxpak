//! `cxpak schema` — print the JSON Schema for the `auto_context` output contract.
//!
//! The schema is versioned by [`crate::auto_context::FORMAT_VERSION`] (ADR-0169):
//! consumers can pin to a `format_version` and detect breaking changes. It documents
//! the existing JSON output rather than introducing a new serialization format.

use serde_json::json;

/// Build the JSON Schema document for `AutoContextResult`, keyed to the current
/// `FORMAT_VERSION`. Kept in one place so the advertised contract and the const
/// never drift.
pub fn auto_context_schema() -> serde_json::Value {
    let version = crate::auto_context::FORMAT_VERSION;
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
                "description": "Packed context sections (target/test/schema files, API surface, blast radius).",
                "properties": {
                    "target_files": { "$ref": "#/$defs/packedFileSection" },
                    "test_files": { "$ref": "#/$defs/packedFileSection" },
                    "schema_context": { "$ref": "#/$defs/packedFileSection" }
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

/// Print the schema as pretty JSON to stdout.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let schema = auto_context_schema();
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
}
