//! KME data model. Mirrors `docs/memory/KODO_MEMORY_IMPLEMENTATION.md` §6 / §7
//! (six memory kinds, state machine, scope hierarchy, authority levels,
//! evidence references, outbox events, usage feedback).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The six memory kinds. The string is stable on disk and in JSON; do not
/// rename without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    /// Current task goal, todo, next step, blocker. Lifetime = session/task.
    Working,
    /// Subtask experience: context, problem, attempts, root cause, fix.
    /// Lifetime = repo + branch (or worktree).
    Episodic,
    /// Architecture, module relationships, technical facts confirmed by
    /// evidence. Lifetime = repo/project.
    Semantic,
    /// Tool usage, fix steps, reusable workflow.
    /// Lifetime = project/user (controlled).
    Procedural,
    /// User-stated preferences / project constraints. Lifetime = user/project.
    Rule,
    /// Pointers to evidence (session log, file, commit, test, diff).
    /// Lifetime = same as the source.
    EvidenceRef,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Working => "working",
            MemoryKind::Episodic => "episodic",
            MemoryKind::Semantic => "semantic",
            MemoryKind::Procedural => "procedural",
            MemoryKind::Rule => "rule",
            MemoryKind::EvidenceRef => "evidence_ref",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "working" => MemoryKind::Working,
            "episodic" => MemoryKind::Episodic,
            "semantic" => MemoryKind::Semantic,
            "procedural" => MemoryKind::Procedural,
            "rule" => MemoryKind::Rule,
            "evidence_ref" => MemoryKind::EvidenceRef,
            _ => return None,
        })
    }

    /// Default scope for this kind. Matches doc §6.1.
    pub fn default_scope(self) -> ScopeLayer {
        match self {
            MemoryKind::Working => ScopeLayer::Session,
            MemoryKind::Episodic => ScopeLayer::Branch,
            MemoryKind::Semantic => ScopeLayer::Project,
            MemoryKind::Procedural => ScopeLayer::Project,
            MemoryKind::Rule => ScopeLayer::Project,
            MemoryKind::EvidenceRef => ScopeLayer::Project,
        }
    }
}

/// Memory status. The state machine in doc §6.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// Newly written, not yet vetted by the value gate.
    Candidate,
    /// Live, used for recall.
    Active,
    /// Underlying file/code changed; needs revalidation.
    Stale,
    /// Replaced by a newer entry; kept for history.
    Superseded,
    /// Held for human review; excluded from recall.
    Quarantined,
    /// Permanently excluded; not loaded.
    Rejected,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Candidate => "candidate",
            MemoryStatus::Active => "active",
            MemoryStatus::Stale => "stale",
            MemoryStatus::Superseded => "superseded",
            MemoryStatus::Quarantined => "quarantined",
            MemoryStatus::Rejected => "rejected",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "candidate" => MemoryStatus::Candidate,
            "active" => MemoryStatus::Active,
            "stale" => MemoryStatus::Stale,
            "superseded" => MemoryStatus::Superseded,
            "quarantined" => MemoryStatus::Quarantined,
            "rejected" => MemoryStatus::Rejected,
            _ => return None,
        })
    }
}

/// Authority — does this memory get to influence the model, or is it just
/// historical context? `System` is reserved for entries the user has
/// explicitly approved; the engine never promotes to it on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// Default. Low-authority historical context, never a system instruction.
    #[default]
    Historical,
    /// User-approved rule. Still not a system instruction; surfaced with
    /// higher visibility in the recall pack.
    UserRule,
    /// Reserved for future use; the engine will not produce this today.
    System,
}

impl Authority {
    pub fn as_str(self) -> &'static str {
        match self {
            Authority::Historical => "historical",
            Authority::UserRule => "user_rule",
            Authority::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "historical" => Authority::Historical,
            "user_rule" => Authority::UserRule,
            "system" => Authority::System,
            _ => return None,
        })
    }
}

/// Where the memory came from. Determines whether it can be treated as
/// verified. `UserConfirmed` requires an explicit user gesture (e.g.
/// "remember this" or a configuration write).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrust {
    #[default]
    Untrusted,
    ToolVerified,
    TestVerified,
    UserConfirmed,
}

impl SourceTrust {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceTrust::Untrusted => "untrusted",
            SourceTrust::ToolVerified => "tool_verified",
            SourceTrust::TestVerified => "test_verified",
            SourceTrust::UserConfirmed => "user_confirmed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "untrusted" => SourceTrust::Untrusted,
            "tool_verified" => SourceTrust::ToolVerified,
            "test_verified" => SourceTrust::TestVerified,
            "user_confirmed" => SourceTrust::UserConfirmed,
            _ => return None,
        })
    }

    /// Whether this trust level is enough to mark the entry verified.
    pub fn counts_as_verified(self) -> bool {
        matches!(
            self,
            SourceTrust::ToolVerified | SourceTrust::TestVerified | SourceTrust::UserConfirmed
        )
    }
}

/// Scope hierarchy. Hard filter at recall time (doc §6.3). A memory's
/// effective scope is the set of `(project_id, repository_id, branch,
/// worktree_id, session_id, task_id)` tuples it is allowed to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeLayer {
    UserLocal,
    Project,
    Repository,
    Branch,
    Worktree,
    Session,
}

impl ScopeLayer {
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeLayer::UserLocal => "user_local",
            ScopeLayer::Project => "project",
            ScopeLayer::Repository => "repository",
            ScopeLayer::Branch => "branch",
            ScopeLayer::Worktree => "worktree",
            ScopeLayer::Session => "session",
        }
    }
}

/// Identity fields that bucket memory recall and isolation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryScope {
    /// Stable project id (not the display name). Local, not shared with the
    /// remote. `None` for project-less Work mode.
    pub project_id: Option<String>,
    /// Git common-dir absolute path. None when the project has no git.
    pub repository_id: Option<String>,
    /// Branch name. None when the project has no git.
    pub branch: Option<String>,
    /// Worktree path (as reported by `git worktree list --porcelain`).
    /// None when not in a worktree.
    pub worktree_id: Option<String>,
    /// Session id; lives only for the duration of a conversation.
    pub session_id: Option<String>,
    /// Task plan id; can persist across sessions.
    pub task_id: Option<String>,
    /// Subtask id; smaller than task_id.
    pub subtask_id: Option<String>,
}

impl MemoryScope {
    /// True when this scope overlaps the caller's scope. A session-level
    /// memory matches the same session only. A branch-level memory matches
    /// any session in the same branch. A project-level memory matches any
    /// session in the same project.
    pub fn matches(&self, caller: &MemoryScope) -> bool {
        if let (Some(a), Some(b)) = (&self.project_id, &caller.project_id) {
            if a != b {
                return false;
            }
        } else if self.project_id.is_some() || caller.project_id.is_some() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.repository_id, &caller.repository_id) {
            if a != b {
                return false;
            }
        } else if self.repository_id.is_some() || caller.repository_id.is_some() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.worktree_id, &caller.worktree_id) {
            if a != b {
                return false;
            }
        } else if self.worktree_id.is_some() || caller.worktree_id.is_some() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.session_id, &caller.session_id) {
            if a != b {
                return false;
            }
        } else if self.session_id.is_some() || caller.session_id.is_some() {
            return false;
        }
        if let (Some(a), Some(b)) = (&self.branch, &caller.branch) {
            if a != b {
                return false;
            }
        } else if self.branch.is_some() || caller.branch.is_some() {
            return false;
        }
        true
    }
}

/// One memory row. The body is a JSON value (kind-specific). Title +
/// abstract are flattened so FTS5 can search them without unpacking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub id: String,
    pub kind: MemoryKind,
    pub status: MemoryStatus,
    pub authority: Authority,
    pub scope: MemoryScope,
    pub title: String,
    pub abstract_text: String,
    pub body: serde_json::Value,
    pub content_hash: String,
    pub source_trust: SourceTrust,
    pub confidence: f32,
    pub verified: bool,
    /// Optional free-form conditions that must hold for the memory to apply.
    pub conditions: Vec<String>,
    /// Optional proof references. URIs use the `kodo://` or `git://` schemes.
    pub source_uris: Vec<String>,
    /// Optional validity window in epoch seconds.
    pub valid_from: Option<u64>,
    pub valid_until: Option<u64>,
    pub last_verified_at: Option<u64>,
    pub last_used_at: Option<u64>,
    pub use_count: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub superseded_by: Option<String>,
    pub version: u32,
    /// P2: marks procedural entries that are candidates to become Skills.
    #[serde(default)]
    pub candidate_skill: bool,
}

impl MemoryEntry {
    /// Whether this entry is allowed to be returned by `recall` for a
    /// general agent run. Excludes `quarantined`, `rejected`, and
    /// `superseded` (unless history mode is on).
    pub fn visible_to_recall(&self) -> bool {
        matches!(self.status, MemoryStatus::Candidate | MemoryStatus::Active)
    }
}

/// Filter for `list`/`search` commands.
#[derive(Debug, Clone, Default)]
pub struct MemoryFilter {
    pub scope: Option<MemoryScope>,
    pub kind: Option<MemoryKind>,
    pub status: Option<MemoryStatus>,
    pub project_id: Option<String>,
    pub repository_id: Option<String>,
    pub limit: Option<usize>,
    pub include_history: bool,
}

/// A single snippet returned in a MemoryPack.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySnippet {
    pub id: String,
    pub kind: MemoryKind,
    pub title: String,
    pub abstract_text: String,
    pub source_uris: Vec<String>,
    pub conditions: Vec<String>,
    /// "historical" or "current evidence" (only when fully verified + same code state).
    pub scope_label: String,
    pub authority: Authority,
    pub verified: bool,
    /// `SameFileDigest` / `CompatibleCommit` / `NeedsRevalidation` / `Stale` /
    /// `HistoricalOnly`. P0 reports `CompatibleCommit` for everything.
    pub validity: ValidityLabel,
}

/// Validity of a memory against the live code/git state. Doc §11.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidityLabel {
    SameFileDigest,
    CompatibleCommit,
    NeedsRevalidation,
    Stale,
    HistoricalOnly,
}

impl ValidityLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            ValidityLabel::SameFileDigest => "same_file_digest",
            ValidityLabel::CompatibleCommit => "compatible_commit",
            ValidityLabel::NeedsRevalidation => "needs_revalidation",
            ValidityLabel::Stale => "stale",
            ValidityLabel::HistoricalOnly => "historical_only",
        }
    }
}

/// The retrieval result. The `omitted_count` is exposed so the agent can
/// say "X other memories were filtered" without exposing their bodies.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryPack {
    pub snippets: Vec<MemorySnippet>,
    pub estimated_tokens: usize,
    pub retrieved_ids: Vec<String>,
    pub omitted_count: usize,
    pub diagnostic: MemoryDiagnostic,
}

impl MemoryPack {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Per-recall diagnostics. Kept stringly-typed so it serializes cleanly to
/// the GUI; do not parse it on the model side.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryDiagnostic {
    pub scope_filter_hits: usize,
    pub fts_hits: usize,
    pub exact_hits: usize,
    pub after_validity_filter: usize,
    pub after_diversity_filter: usize,
    pub tokens_used: usize,
    pub elapsed_ms: u64,
    pub note: String,
}

/// What the agent tells the memory engine about a query.
#[derive(Debug, Clone)]
pub struct MemoryQuery {
    pub scope: MemoryScope,
    /// Free-text query. Usually the current user request.
    pub text: String,
    /// Project-relative paths the user pinned or the model touched. Used to
    /// boost entries that share a file.
    pub related_paths: Vec<String>,
    /// Symbols (functions, types, error codes) the model mentioned.
    pub related_symbols: Vec<String>,
    /// Hard cap on the pack size. The engine tries to keep `estimated_tokens`
    /// at or below this.
    pub budget_tokens: usize,
    /// If true, prefer only `verified` entries. The pack is empty otherwise
    /// unless the caller relaxes the request.
    pub require_current_evidence: bool,
}

impl MemoryQuery {
    /// An empty / disabled query — used to short-circuit when memory is off.
    pub fn inert(scope: MemoryScope) -> Self {
        Self {
            scope,
            text: String::new(),
            related_paths: Vec::new(),
            related_symbols: Vec::new(),
            budget_tokens: 0,
            require_current_evidence: false,
        }
    }
}

/// A sanitized event the agent loop delivers to the outbox. It is a
/// conservative subset of `kodo_core::session::ItemKind` — only the fields
/// the memory engine is allowed to read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SafeMemoryEvent {
    pub event_id: String,
    pub session_id: String,
    pub turn_seq: u32,
    pub item_seq: u32,
    pub phase: EventPhase,
    pub kind: MemoryEventKind,
    pub source_digest: String,
    pub observed_at: u64,
    /// Project-relative path, when applicable (file reads, edits, searches).
    pub project_path: Option<String>,
    /// Model or tool name for context.
    pub label: String,
    /// Optional body, already redacted.
    pub body: Option<String>,
    /// Free-form fields the agent may add; rejected if not in allowlist.
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase {
    Started,
    Completed,
    Failed,
    Blocked,
}

impl EventPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            EventPhase::Started => "started",
            EventPhase::Completed => "completed",
            EventPhase::Failed => "failed",
            EventPhase::Blocked => "blocked",
        }
    }
}

/// What kind of source event this represents. Mirrors the kinds the
/// `ItemKind` enum exposes, trimmed to the high-signal subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEventKind {
    Reasoning,
    Search,
    FileRead,
    FileChange,
    CommandExecution,
    ModelCall,
    AgentMessage,
    Compaction,
    Failover,
    PlanStep,
    Permission,
    ContextBudget,
}

impl MemoryEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryEventKind::Reasoning => "reasoning",
            MemoryEventKind::Search => "search",
            MemoryEventKind::FileRead => "file_read",
            MemoryEventKind::FileChange => "file_change",
            MemoryEventKind::CommandExecution => "command_execution",
            MemoryEventKind::ModelCall => "model_call",
            MemoryEventKind::AgentMessage => "agent_message",
            MemoryEventKind::Compaction => "compaction",
            MemoryEventKind::Failover => "failover",
            MemoryEventKind::PlanStep => "plan_step",
            MemoryEventKind::Permission => "permission",
            MemoryEventKind::ContextBudget => "context_budget",
        }
    }
}

/// Per-recall usage feedback. P0 just stores it; P2 reads it for policy
/// tuning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryUsage {
    pub run_id: String,
    pub memory_id: String,
    pub operation: MemoryOperation,
    pub score: Option<f32>,
    pub token_cost: Option<u32>,
    pub elapsed_ms: Option<u32>,
    pub at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOperation {
    Retrieved,
    Packed,
    Inspected,
    Helpful,
    Harmful,
}

impl MemoryOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryOperation::Retrieved => "retrieved",
            MemoryOperation::Packed => "packed",
            MemoryOperation::Inspected => "inspected",
            MemoryOperation::Helpful => "helpful",
            MemoryOperation::Harmful => "harmful",
        }
    }
}

/// Configuration read from settings.log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub mode: MemoryMode,
    pub max_pack_tokens: u32,
    pub auto_extract: bool,
    pub global_scope: bool,
    pub embedding_provider: EmbeddingBackend,
    pub retention_days: u32,
    /// `high` blocks secret-like bodies; `low` allows them with a warning.
    pub privacy: MemoryPrivacy,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: MemoryMode::Off,
            max_pack_tokens: 2000,
            auto_extract: false,
            global_scope: false,
            embedding_provider: EmbeddingBackend::Off,
            retention_days: 365,
            privacy: MemoryPrivacy::High,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryMode {
    Off,
    /// Read paths run; nothing is written to the DB.
    Shadow,
    /// Read + write.
    ReadWrite,
}

/// Embedding backend. `Off` is the default and keeps recall running on
/// FTS + exact-token + RRF alone; the embedding channel is simply
/// skipped. `Local*` variants require an explicit warmup that downloads
/// model weights to a local cache — they never auto-download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingBackend {
    /// No embedding channel. RRF runs on FTS + exact only.
    Off,
    /// `BAAI/bge-small-en-v1.5` — 384-dim, MIT, ~33 MB.
    /// English-leaning code / docs. Opt-in, warmup required.
    LocalBgeSmallEn,
    /// `BAAI/bge-small-zh-v1.5` — 512-dim, MIT, ~47 MB.
    /// Chinese-leaning content. Opt-in, warmup required.
    LocalBgeSmallZh,
    /// `sentence-transformers/all-MiniLM-L6-v2` — 384-dim, Apache-2.0,
    /// ~23 MB. Smallest local option, weakest of the three. Opt-in.
    LocalMiniLmL6,
    /// Offline trigram hash. No model download, no semantic value;
    /// exists only so the RRF wiring path is exercised in tests.
    LocalHash,
}

impl EmbeddingBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            EmbeddingBackend::Off => "off",
            EmbeddingBackend::LocalBgeSmallEn => "local_bge_small_en",
            EmbeddingBackend::LocalBgeSmallZh => "local_bge_small_zh",
            EmbeddingBackend::LocalMiniLmL6 => "local_minilm_l6",
            EmbeddingBackend::LocalHash => "local_hash",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "off" | "" => EmbeddingBackend::Off,
            // Canonical snake_case names, plus the friendly short
            // forms (without underscores) so `bge-small-en` is a
            // valid CLI value.
            "local_bge_small_en" | "bge-small-en" | "bge_small_en" => {
                EmbeddingBackend::LocalBgeSmallEn
            }
            "local_bge_small_zh" | "bge-small-zh" | "bge_small_zh" => {
                EmbeddingBackend::LocalBgeSmallZh
            }
            "local_minilm_l6" | "minilm-l6" | "minilm_l6" => EmbeddingBackend::LocalMiniLmL6,
            "local_hash" | "hash" => EmbeddingBackend::LocalHash,
            _ => return None,
        })
    }

    /// True when this backend needs a model warmup (network download +
    /// on-disk cache). `Off` and `LocalHash` are warmup-free.
    pub fn requires_warmup(self) -> bool {
        matches!(
            self,
            EmbeddingBackend::LocalBgeSmallEn
                | EmbeddingBackend::LocalBgeSmallZh
                | EmbeddingBackend::LocalMiniLmL6
        )
    }

    /// Model id used for download. `None` for warmup-free backends.
    pub fn model_id(self) -> Option<&'static str> {
        match self {
            EmbeddingBackend::LocalBgeSmallEn => Some("Xenova/bge-small-en-v1.5"),
            EmbeddingBackend::LocalBgeSmallZh => Some("Xenova/bge-small-zh-v1.5"),
            EmbeddingBackend::LocalMiniLmL6 => Some("Xenova/all-MiniLM-L6-v2"),
            EmbeddingBackend::Off | EmbeddingBackend::LocalHash => None,
        }
    }

    /// Output embedding dimension. 0 for `Off` (skipped channel).
    pub fn dim(self) -> usize {
        match self {
            EmbeddingBackend::Off => 0,
            EmbeddingBackend::LocalBgeSmallEn => 384,
            EmbeddingBackend::LocalBgeSmallZh => 512,
            EmbeddingBackend::LocalMiniLmL6 => 384,
            EmbeddingBackend::LocalHash => 64,
        }
    }

    /// HuggingFace repo id used to resolve direct URLs. Kept in sync
    /// with `model_id()` so the warmup path can build the download URL
    /// without a second source of truth.
    pub fn hf_repo(self) -> Option<&'static str> {
        match self {
            EmbeddingBackend::LocalBgeSmallEn => Some("Xenova/bge-small-en-v1.5"),
            EmbeddingBackend::LocalBgeSmallZh => Some("Xenova/bge-small-zh-v1.5"),
            EmbeddingBackend::LocalMiniLmL6 => Some("Xenova/all-MiniLM-L6-v2"),
            EmbeddingBackend::Off | EmbeddingBackend::LocalHash => None,
        }
    }

    /// Whether the warmup path is wired for this backend.
    /// `LocalBgeSmallEn` is the first fully-wired backend. The other
    /// local variants return `None` for `hf_repo()` until a follow-up
    /// PR pins their URLs and verifies the SHA-256s.
    pub fn has_download_path(self) -> bool {
        self.hf_repo().is_some() && matches!(self, EmbeddingBackend::LocalBgeSmallEn)
    }
}

impl MemoryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryMode::Off => "off",
            MemoryMode::Shadow => "shadow",
            MemoryMode::ReadWrite => "read_write",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "off" => MemoryMode::Off,
            "shadow" => MemoryMode::Shadow,
            "read_write" => MemoryMode::ReadWrite,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPrivacy {
    /// Reject secret-like bodies; refuse to recall quarantined entries.
    High,
    /// Allow secret-like bodies in the DB but redact in recall.
    Low,
}

impl MemoryPrivacy {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryPrivacy::High => "high",
            MemoryPrivacy::Low => "low",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "high" => MemoryPrivacy::High,
            "low" => MemoryPrivacy::Low,
            _ => return None,
        })
    }
}
