use serde::{Deserialize, Serialize};

/// Confidence level for a resolved call edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallConfidence {
    /// Tree-sitter extracted call expression, import-resolved to a specific file.
    Exact,
    /// Regex-matched against known symbol names in Tier 2 or unresolvable Tier 1.
    Approximate,
}

/// A resolved cross-file function call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEdge {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_file: String,
    pub callee_symbol: String,
    pub confidence: CallConfidence,
}

/// A call that could not be resolved to a specific file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedCall {
    pub caller_file: String,
    pub caller_symbol: String,
    pub callee_name: String,
}

/// The full call graph for a codebase.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<UnresolvedCall>,
}

impl CallGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns all callers of a given symbol in a given file.
    pub fn callers_of(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.callee_file == file && e.callee_symbol == symbol)
            .collect()
    }

    /// Returns all callees from a given symbol in a given file.
    pub fn callees_from(&self, file: &str, symbol: &str) -> Vec<&CallEdge> {
        self.edges
            .iter()
            .filter(|e| e.caller_file == file && e.caller_symbol == symbol)
            .collect()
    }

    /// Returns true if a symbol has at least one caller.
    pub fn has_callers(&self, file: &str, symbol: &str) -> bool {
        self.edges
            .iter()
            .any(|e| e.callee_file == file && e.callee_symbol == symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_graph_default_is_empty() {
        let cg = CallGraph::default();
        assert!(cg.edges.is_empty());
        assert!(cg.unresolved.is_empty());
    }

    #[test]
    fn test_callers_of_returns_matching_edges() {
        let cg = CallGraph {
            edges: vec![
                CallEdge {
                    caller_file: "a.rs".into(),
                    caller_symbol: "foo".into(),
                    callee_file: "b.rs".into(),
                    callee_symbol: "bar".into(),
                    confidence: CallConfidence::Exact,
                },
                CallEdge {
                    caller_file: "c.rs".into(),
                    caller_symbol: "baz".into(),
                    callee_file: "b.rs".into(),
                    callee_symbol: "bar".into(),
                    confidence: CallConfidence::Approximate,
                },
            ],
            unresolved: vec![],
        };
        let callers = cg.callers_of("b.rs", "bar");
        assert_eq!(callers.len(), 2);
    }

    #[test]
    fn test_has_callers_false_for_unknown_symbol() {
        let cg = CallGraph::default();
        assert!(!cg.has_callers("any.rs", "unknown"));
    }

    #[test]
    fn test_callees_from_returns_matching_edges() {
        let cg = CallGraph {
            edges: vec![CallEdge {
                caller_file: "a.rs".into(),
                caller_symbol: "main".into(),
                callee_file: "b.rs".into(),
                callee_symbol: "init".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: vec![],
        };
        let callees = cg.callees_from("a.rs", "main");
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].callee_symbol, "init");
    }

    #[test]
    fn test_call_graph_new_equals_default() {
        let cg = CallGraph::new();
        assert!(cg.edges.is_empty());
        assert!(cg.unresolved.is_empty());
    }

    #[test]
    fn test_call_graph_serialize_deserialize() {
        let cg = CallGraph {
            edges: vec![CallEdge {
                caller_file: "a.rs".into(),
                caller_symbol: "foo".into(),
                callee_file: "b.rs".into(),
                callee_symbol: "bar".into(),
                confidence: CallConfidence::Exact,
            }],
            unresolved: vec![UnresolvedCall {
                caller_file: "a.rs".into(),
                caller_symbol: "foo".into(),
                callee_name: "unknown_fn".into(),
            }],
        };
        let json = serde_json::to_string(&cg).unwrap();
        let restored: CallGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.edges.len(), 1);
        assert_eq!(restored.unresolved.len(), 1);
    }
}
