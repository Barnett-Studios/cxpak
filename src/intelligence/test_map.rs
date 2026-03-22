use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestFileRef {
    pub path: String,
    pub confidence: TestConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestConfidence {
    NameMatch,
    ImportMatch,
    Both,
}
