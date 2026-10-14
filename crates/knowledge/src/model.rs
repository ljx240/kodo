use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeBase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub project_path: Option<String>,
    pub created_at: u64,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    Ready,
    Disabled,
    Quarantined,
}

impl DocumentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Disabled => "disabled",
            Self::Quarantined => "quarantined",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "disabled" => Self::Disabled,
            "quarantined" => Self::Quarantined,
            _ => Self::Ready,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Note,
    File,
    Result,
    Answer,
}

impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::File => "file",
            Self::Result => "result",
            Self::Answer => "answer",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "file" => Self::File,
            "result" => Self::Result,
            "answer" => Self::Answer,
            _ => Self::Note,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeDocument {
    pub id: String,
    pub base_id: String,
    pub title: String,
    pub source_kind: SourceKind,
    pub source_ref: Option<String>,
    pub status: DocumentStatus,
    pub verified: bool,
    pub version: u64,
    pub revision_id: String,
    pub content_hash: String,
    pub chunk_count: usize,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDraft {
    pub title: String,
    pub body: String,
    pub source_kind: SourceKind,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeChunk {
    pub id: String,
    pub revision_id: String,
    pub heading: String,
    pub line_start: usize,
    pub line_end: usize,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDetail {
    pub document: KnowledgeDocument,
    pub body: String,
    pub chunks: Vec<KnowledgeChunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    #[serde(default)]
    pub base_ids: Vec<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    pub text: String,
    pub limit: usize,
    pub budget_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeHit {
    pub base_id: String,
    pub document_id: String,
    pub revision_id: String,
    pub chunk_id: String,
    pub title: String,
    pub heading: String,
    pub line_start: usize,
    pub line_end: usize,
    pub quote: String,
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSelection {
    #[serde(default)]
    pub base_ids: Vec<String>,
    #[serde(default = "default_strict")]
    pub strict: bool,
}

fn default_strict() -> bool {
    true
}

impl Default for KnowledgeSelection {
    fn default() -> Self {
        Self {
            base_ids: Vec::new(),
            strict: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnKnowledge {
    pub session_id: String,
    pub turn_seq: u64,
    pub hits: Vec<KnowledgeHit>,
}
