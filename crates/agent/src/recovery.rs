//! Private engine state; never reachable through model file tools.
use crate::{
    checkpoint::TurnChangeSet,
    context::ContextManager,
    evidence::EvidenceKind,
    plan::{AcceptanceEvidence, TaskPlan},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) fn state_path(project: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || name.len() > 180
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
        || name.starts_with('.')
    {
        return Err("invalid engine state filename".into());
    }
    let root = project.canonicalize().map_err(|e| e.to_string())?;
    let dir = root.join(".kodo");
    for path in [&dir, &dir.join(name)] {
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("engine state symlink is forbidden".into());
        }
    }
    Ok(dir.join(name))
}

pub(crate) fn write_state(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or("state has no parent")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let temp = dir.join(format!(
        ".state-{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}

#[derive(Serialize, Deserialize)]
pub(crate) struct RecoveryCheckpoint {
    version: u32,
    pub plan: TaskPlan,
    pub evidence: AcceptanceEvidence,
    pub state: String,
    file_versions: BTreeMap<String, String>,
}

impl RecoveryCheckpoint {
    pub fn save(
        project: &Path,
        session: Option<&str>,
        plan: &TaskPlan,
        evidence: &AcceptanceEvidence,
        state: &str,
        changes: &TurnChangeSet,
    ) -> Result<(), String> {
        let Some(session) = session else {
            return Ok(());
        };
        let context = ContextManager::new(project, Default::default());
        let mut paths: Vec<String> = changes.kodo_touched.iter().cloned().collect();
        for item in &evidence.bag.items {
            if let EvidenceKind::FileRead { path }
            | EvidenceKind::FileChanged { path }
            | EvidenceKind::PatchApplied { path }
            | EvidenceKind::ContextObservation { path } = &item.kind
            {
                paths.push(path.clone());
            }
        }
        paths.sort();
        paths.dedup();
        paths.truncate(2000);
        let file_versions = paths
            .into_iter()
            .map(|path| {
                let hash = context.file_version(&path);
                (path, hash)
            })
            .collect();
        let checkpoint = Self {
            version: 1,
            plan: plan.clone(),
            evidence: evidence.clone(),
            state: state.into(),
            file_versions,
        };
        let bytes = serde_json::to_vec(&checkpoint).map_err(|e| e.to_string())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("recovery checkpoint exceeds 2 MiB".into());
        }
        write_state(
            &state_path(project, &format!("checkpoint-{session}.json"))?,
            &bytes,
        )
    }

    pub fn load(project: &Path, session: &str) -> Result<Option<Self>, String> {
        let path = state_path(project, &format!("checkpoint-{session}.json"))?;
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        if meta.len() > 2 * 1024 * 1024 {
            return Err("recovery checkpoint exceeds 2 MiB".into());
        }
        let checkpoint: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if checkpoint.version != 1 {
            return Err("unsupported recovery checkpoint version".into());
        }
        Ok(Some(checkpoint))
    }

    pub fn revalidate(mut self, project: &Path) -> (TaskPlan, String) {
        let context = ContextManager::new(project, Default::default());
        let stale: Vec<_> = self
            .file_versions
            .iter()
            .filter(|(path, hash)| context.file_version(path) != **hash)
            .map(|(path, _)| path.as_str())
            .collect();
        let anchor = format!("Recovered state: {}. Previously observed files: {:?}. Changed since checkpoint: {:?}. Re-read targets; all completion and command verification evidence must be renewed.", self.state, self.file_versions.keys().collect::<Vec<_>>(), stale);
        self.plan.current_subtask = 0;
        self.plan.verify_commands.clear();
        self.plan.verify_bindings.clear();
        self.plan.verify_target_ids.clear();
        for task in &mut self.plan.subtasks {
            task.status = crate::plan::SubtaskStatus::Pending;
            task.evidence.clear();
        }
        for criterion in &mut self.plan.criteria {
            criterion.status = crate::evidence::CriterionStatus::Pending;
            criterion.evidence.clear();
        }
        (self.plan, anchor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoint_preserves_contract_but_invalidates_completed_evidence() {
        let root = std::env::temp_dir().join(format!("kodo-recovery-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("report.md"), "old").unwrap();
        let mut plan = TaskPlan::from_task("write report.md");
        plan.expected_outputs = vec!["report.md".into()];
        plan.constraints.push("preserve user edits".into());
        plan.subtasks[0].status = crate::plan::SubtaskStatus::Done;
        let mut changes = TurnChangeSet::default();
        changes.kodo_touched.insert("report.md".into());
        RecoveryCheckpoint::save(
            &root,
            Some("s-1"),
            &plan,
            &AcceptanceEvidence::default(),
            "Execute",
            &changes,
        )
        .unwrap();
        fs::write(root.join("report.md"), "user edit").unwrap();
        let (recovered, anchor) = RecoveryCheckpoint::load(&root, "s-1")
            .unwrap()
            .unwrap()
            .revalidate(&root);
        assert_eq!(recovered.goal, plan.goal);
        assert_eq!(recovered.expected_outputs, plan.expected_outputs);
        assert_eq!(recovered.constraints, plan.constraints);
        assert_eq!(
            recovered.subtasks[0].status,
            crate::plan::SubtaskStatus::Pending
        );
        assert!(anchor.contains("Changed since checkpoint: [\"report.md\"]"));
        assert!(RecoveryCheckpoint::load(&root, "../../escape").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(state_path(&root, "checkpoint-s-1.json").unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn state_directory_cannot_redirect_to_another_directory() {
        let root =
            std::env::temp_dir().join(format!("kodo-recovery-symlink-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join(".kodo")).unwrap();
        assert!(state_path(&root, "checkpoint-s.json").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
