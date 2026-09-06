//! 文件型在线演进 provider:技能读取、pending lifecycle、evolution log 和 SKILL.md 落盘。

use crate::checkpoint_types::{EvolutionLog, EvolutionPatch, EvolutionRecord};
use crate::experience_query::RecordView;
use crate::experience_types::{
    ExperienceApplyResult, ExperienceApprovalRequest, ExperienceProposal, PendingChange,
};
use crate::online_orchestrator::{OnlineEvolutionManager, OnlineEvolutionStore};
use ah_contracts::effect::Effect;
use ah_contracts::evolving::{ApplyResult, UpdateEffect, UpdateMode, UpdateValue};
use ah_contracts::operator::{Operator, OperatorError, ParameterUpdated, TunableKind, TunableSpec};
use ah_contracts::seam::Seam;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 真实文件演进 store。目录布局为 `{root}/{skill}/SKILL.md` 与 `evolutions.json`。
pub struct FileEvolutionStore {
    root: PathBuf,
}

impl FileEvolutionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn skill_dir(&self, skill_name: &str) -> Result<PathBuf, String> {
        if skill_name.is_empty()
            || skill_name == "."
            || skill_name == ".."
            || skill_name.contains('/')
            || skill_name.contains('\\')
        {
            return Err(format!("invalid skill name: {skill_name}"));
        }
        Ok(self.root.join(skill_name))
    }

    fn skill_file(&self, skill_name: &str) -> Result<PathBuf, String> {
        Ok(self.skill_dir(skill_name)?.join("SKILL.md"))
    }

    fn log_file(&self, skill_name: &str) -> Result<PathBuf, String> {
        Ok(self.skill_dir(skill_name)?.join("evolutions.json"))
    }

    fn load_log(&self, skill_name: &str) -> Result<EvolutionLog, String> {
        let path = self.log_file(skill_name)?;
        if !path.exists() {
            return Ok(EvolutionLog::empty(skill_name));
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("read evolution log {}: {e}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("parse evolution log {}: {e}", path.display()))?;
        EvolutionLog::from_dict(
            value
                .as_object()
                .ok_or_else(|| format!("evolution log is not an object: {}", path.display()))?,
        )
        .map_err(|e| format!("decode evolution log {}: {e}", path.display()))
    }

    fn record_view(record: &EvolutionRecord) -> RecordView {
        RecordView {
            id: record.id.clone(),
            summary: record.summary.clone(),
            score: record.score,
            timestamp: record.timestamp.clone(),
            target: record.change.target.clone(),
            section: record.change.section.clone(),
            content: record.change.content.clone(),
        }
    }

    fn render_skill(content: &str, records: &[RecordView]) -> Result<String, String> {
        let mut rendered = content.to_string();
        for record in records {
            match record.target.as_str() {
                "body" => {
                    rendered.push_str("\n\n## Evolution Experience\n\n");
                    rendered.push_str(&format!("- {}\n", record.content));
                }
                "description" => {
                    let Some(line_end) = rendered.find('\n') else {
                        return Err("SKILL.md has no description frontmatter".to_string());
                    };
                    let prefix = &rendered[..line_end];
                    if !prefix.trim().starts_with("---") {
                        return Err("description update requires SKILL.md frontmatter".to_string());
                    }
                    let Some(description_start) = rendered
                        .lines()
                        .position(|line| line.trim_start().starts_with("description:"))
                    else {
                        return Err("SKILL.md has no description field".to_string());
                    };
                    let mut lines: Vec<String> = rendered.lines().map(str::to_string).collect();
                    lines[description_start].push(' ');
                    lines[description_start].push_str(&record.content);
                    rendered = lines.join("\n");
                    rendered.push('\n');
                }
                "script" => {
                    let filename = Self::script_filename(&record.id);
                    rendered.push_str("\n\n## Script Assets\n\n");
                    rendered.push_str(&format!(
                        "- [{}](evolution/scripts/{filename})\n",
                        record.summary.as_deref().unwrap_or(&record.id)
                    ));
                }
                other => return Err(format!("unsupported SKILL.md target: {other}")),
            }
        }
        Ok(rendered)
    }

    fn script_filename(record_id: &str) -> String {
        let safe: String = record_id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        format!("{}.md", if safe.is_empty() { "script" } else { &safe })
    }

    fn atomic_write(path: &Path, content: &[u8], suffix: &str) -> Result<(), String> {
        let temp = path.with_extension(format!(
            "{}.tmp-{}",
            path.extension().and_then(|s| s.to_str()).unwrap_or("file"),
            suffix
        ));
        std::fs::write(&temp, content)
            .map_err(|e| format!("write temporary file {}: {e}", temp.display()))?;
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(format!("replace {}: {error}", path.display()));
        }
        Ok(())
    }

    fn pending_dir(&self) -> PathBuf {
        self.root.join(".pending")
    }

    fn pending_path(&self, request_id: &str) -> Result<PathBuf, String> {
        if request_id.is_empty()
            || request_id == "."
            || request_id == ".."
            || request_id.contains('/')
            || request_id.contains('\\')
        {
            return Err(format!("invalid pending change id: {request_id}"));
        }
        Ok(self.pending_dir().join(format!("{request_id}.json")))
    }

    fn save_pending(&self, pending: &PendingChange) -> Result<(), String> {
        let path = self.pending_path(&pending.change_id)?;
        std::fs::create_dir_all(self.pending_dir())
            .map_err(|e| format!("create pending directory: {e}"))?;
        let bytes = serde_json::to_vec_pretty(pending)
            .map_err(|e| format!("serialize pending change: {e}"))?;
        Self::atomic_write(&path, &bytes, &format!("{}-pending", std::process::id()))
    }

    fn load_pending(&self) -> Result<BTreeMap<String, PendingChange>, String> {
        let mut pending = BTreeMap::new();
        if !self.pending_dir().exists() {
            return Ok(pending);
        }
        for entry in std::fs::read_dir(self.pending_dir())
            .map_err(|e| format!("read pending directory: {e}"))?
        {
            let entry = entry.map_err(|e| format!("read pending entry: {e}"))?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let path = entry.path();
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("read pending {}: {e}", path.display()))?;
            let item: PendingChange = serde_json::from_str(&text)
                .map_err(|e| format!("parse pending {}: {e}", path.display()))?;
            pending.insert(item.change_id.clone(), item);
        }
        Ok(pending)
    }

    fn remove_pending(&self, request_id: &str) -> Result<(), String> {
        let path = self.pending_path(request_id)?;
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("remove pending {}: {e}", path.display()))?;
        }
        Ok(())
    }

    /// 将已批准记录一次性写入 evolution log 与 SKILL.md。
    pub fn append_records(
        &self,
        skill_name: &str,
        records: &[RecordView],
        source: &str,
    ) -> Result<(), String> {
        if records.is_empty() {
            return Ok(());
        }
        let skill_path = self.skill_file(skill_name)?;
        let log_path = self.log_file(skill_name)?;
        let skill_content = std::fs::read_to_string(&skill_path)
            .map_err(|e| format!("read skill {}: {e}", skill_path.display()))?;
        let rendered_skill = Self::render_skill(&skill_content, records)?;
        let mut log = self.load_log(skill_name)?;
        let mut new_records = Vec::with_capacity(records.len());
        for record in records {
            let is_script = record.target == "script";
            let script_filename = is_script.then(|| Self::script_filename(&record.id));
            let script_purpose = is_script.then(|| record.summary.clone().unwrap_or_default());
            let patch = EvolutionPatch::new(
                record.section.clone(),
                "append".to_string(),
                record.content.clone(),
                record.target.clone(),
                None,
                None,
                script_filename,
                is_script.then(|| "text".to_string()),
                script_purpose,
            )
            .map_err(|e| format!("build evolution patch {}: {e}", record.id))?;
            let mut stored = EvolutionRecord::make(
                source,
                record.summary.as_deref().unwrap_or(&record.content),
                patch,
                record.score,
                None,
                record.summary.clone(),
            );
            stored.applied = true;
            new_records.push(stored);
        }
        log.entries.extend(new_records);
        log.updated_at = crate::experience_types::utc_iso_now();
        let log_text = serde_json::to_vec_pretty(&log.to_dict())
            .map_err(|e| format!("serialize evolution log: {e}"))?;
        std::fs::create_dir_all(skill_path.parent().unwrap_or(&self.root))
            .map_err(|e| format!("create skill directory: {e}"))?;
        let mut script_backups: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
        for record in records.iter().filter(|record| record.target == "script") {
            let script_path = skill_path
                .parent()
                .unwrap_or(&self.root)
                .join("evolution")
                .join("scripts")
                .join(Self::script_filename(&record.id));
            let previous = if script_path.exists() {
                Some(
                    std::fs::read(&script_path)
                        .map_err(|e| format!("read script {}: {e}", script_path.display()))?,
                )
            } else {
                None
            };
            if let Some(parent) = script_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create script directory: {e}"))?;
            }
            let suffix = format!("{}-{}", std::process::id(), log.entries.len());
            if let Err(error) = Self::atomic_write(&script_path, record.content.as_bytes(), &suffix)
            {
                for (path, old) in script_backups {
                    match old {
                        Some(bytes) => {
                            let _ = std::fs::write(path, bytes);
                        }
                        None => {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
                return Err(error);
            }
            script_backups.push((script_path, previous));
        }
        let suffix = format!("{}-{}", std::process::id(), log.entries.len());
        if let Err(error) = Self::atomic_write(&skill_path, rendered_skill.as_bytes(), &suffix) {
            for (path, old) in script_backups {
                match old {
                    Some(bytes) => {
                        let _ = std::fs::write(path, bytes);
                    }
                    None => {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            return Err(error);
        }
        if let Err(error) = Self::atomic_write(&log_path, &log_text, &suffix) {
            // Restore the previous skill content before exposing a failed commit.
            let _ = std::fs::write(&skill_path, skill_content);
            for (path, old) in script_backups {
                match old {
                    Some(bytes) => {
                        let _ = std::fs::write(path, bytes);
                    }
                    None => {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            return Err(error);
        }
        Ok(())
    }
}

impl OnlineEvolutionStore for FileEvolutionStore {
    fn skill_exists(&self, skill_name: &str) -> bool {
        self.skill_dir(skill_name)
            .map(|path| path.is_dir())
            .unwrap_or(false)
    }

    fn skill_definition_exists(&self, skill_name: &str) -> bool {
        self.skill_file(skill_name)
            .map(|path| path.is_file())
            .unwrap_or(false)
    }

    fn skill_content(&self, skill_name: &str) -> Result<String, String> {
        let path = self.skill_file(skill_name)?;
        std::fs::read_to_string(&path).map_err(|e| format!("read skill {}: {e}", path.display()))
    }

    fn pending_records(&self, skill_name: &str, target: &str) -> Result<Vec<RecordView>, String> {
        Ok(self
            .load_log(skill_name)?
            .entries
            .iter()
            .filter(|record| record.is_pending() && record.change.target == target)
            .map(Self::record_view)
            .collect())
    }
}

/// 文件技能对应的经验预览 operator;通过 registry 提供给在线 orchestrator。
pub struct FileSkillExperienceOperator {
    skill_name: String,
}

impl FileSkillExperienceOperator {
    pub fn new(skill_name: impl Into<String>) -> Self {
        Self {
            skill_name: skill_name.into(),
        }
    }
}

impl Seam for FileSkillExperienceOperator {}

impl Operator for FileSkillExperienceOperator {
    fn operator_id(&self) -> String {
        format!("skill_experience_{}", self.skill_name)
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        vec![TunableSpec {
            name: "experiences".to_string(),
            kind: TunableKind::SkillExperience,
            path: "content".to_string(),
            constraint: Some(json!({"type": "record"})),
        }]
    }

    fn get_state(&self) -> Value {
        json!({})
    }

    fn set_parameter(&self, _target: &str, _value: Value) -> Result<(), OperatorError> {
        Ok(())
    }

    fn load_state(&self, _state: Value) -> Result<(), OperatorError> {
        Ok(())
    }

    fn on_parameter_updated(&self, _callback: ParameterUpdated) -> Effect {
        Effect::new(|| {})
    }

    fn apply_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
        let records = match target {
            "experiences"
                if matches!(update.mode, UpdateMode::Append | UpdateMode::Merge)
                    && update.effect == UpdateEffect::PendingChange =>
            {
                match update.payload.as_array() {
                    Some(records) => records.clone(),
                    None => vec![update.payload.clone()],
                }
            }
            "experiences" => {
                return ApplyResult {
                    operator_id: self.operator_id(),
                    target: target.to_string(),
                    applied: false,
                    mode: update.mode,
                    effect: update.effect,
                    value: Some(update.payload.clone()),
                    records: vec![],
                    change_type: update.change_type.clone(),
                    lifecycle_stage: None,
                    pending_change_id: None,
                    errors: vec!["unsupported update mode/effect for file skill experience".into()],
                    metadata: update.metadata.clone(),
                };
            }
            other => {
                return ApplyResult {
                    operator_id: self.operator_id(),
                    target: other.to_string(),
                    applied: false,
                    mode: update.mode,
                    effect: update.effect,
                    value: Some(update.payload.clone()),
                    records: vec![],
                    change_type: update.change_type.clone(),
                    lifecycle_stage: None,
                    pending_change_id: None,
                    errors: vec![format!(
                        "unsupported target for file skill experience: {other}"
                    )],
                    metadata: update.metadata.clone(),
                };
            }
        };
        let mut metadata = update.metadata.clone();
        metadata.insert("skill_name".to_string(), json!(self.skill_name));
        ApplyResult {
            operator_id: self.operator_id(),
            target: target.to_string(),
            applied: !records.is_empty(),
            mode: update.mode,
            effect: update.effect,
            value: Some(update.payload.clone()),
            records,
            change_type: update.change_type.clone(),
            lifecycle_stage: Some("local_apply_completed".to_string()),
            pending_change_id: None,
            errors: vec![],
            metadata,
        }
    }
}

/// 文件型在线生命周期 manager。审批快照同时保存在内存与 `.pending` 文件。
pub struct FileEvolutionManager {
    store: Arc<FileEvolutionStore>,
    pending: Mutex<BTreeMap<String, PendingChange>>,
}

impl FileEvolutionManager {
    pub fn open(store: Arc<FileEvolutionStore>) -> Result<Self, String> {
        let pending = store.load_pending()?;
        Ok(Self {
            store,
            pending: Mutex::new(pending),
        })
    }
}

impl OnlineEvolutionManager for FileEvolutionManager {
    #[allow(clippy::too_many_arguments)]
    fn stage(
        &self,
        mut pending: PendingChange,
        requires_approval: bool,
        source: &str,
        _request_id_prefix: Option<&str>,
        user_query: Option<&str>,
        signal_type: Option<&str>,
        signal_source: Option<&str>,
        messages: Option<Vec<serde_json::Value>>,
    ) -> Result<ExperienceApprovalRequest, String> {
        pending.messages = messages;
        let request_id = pending.change_id.clone();
        let proposal = ExperienceProposal {
            skill_name: pending.skill_name.clone(),
            records: pending.payload.clone(),
            requires_approval,
            source: source.to_string(),
            user_query: user_query.unwrap_or_default().to_string(),
            signal_type: signal_type.map(str::to_string),
            signal_source: signal_source.map(str::to_string),
        };
        self.store.save_pending(&pending)?;
        self.pending
            .lock()
            .unwrap()
            .insert(request_id.clone(), pending.clone());
        Ok(ExperienceApprovalRequest {
            skill_name: pending.skill_name.clone(),
            proposal,
            pending_change: Some(pending),
            request_id: Some(request_id),
        })
    }

    fn approve(&self, request_id: &str) -> Result<ExperienceApplyResult, String> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or_else(|| format!("pending change not found: {request_id}"))?;
        match self
            .store
            .append_records(&pending.skill_name, &pending.payload, "experience_updater")
        {
            Ok(()) => {
                let applied_count = pending.payload.len();
                if let Err(error) = self.store.remove_pending(request_id) {
                    return Ok(ExperienceApplyResult {
                        skill_name: pending.skill_name,
                        applied_count,
                        errors: vec![error],
                        ..Default::default()
                    });
                }
                Ok(ExperienceApplyResult {
                    skill_name: pending.skill_name,
                    applied_count,
                    ..Default::default()
                })
            }
            Err(error) => {
                let count = pending.payload.len();
                self.pending
                    .lock()
                    .unwrap()
                    .insert(request_id.to_string(), pending.clone());
                Ok(ExperienceApplyResult {
                    skill_name: pending.skill_name,
                    pending_count: count,
                    errors: vec![error],
                    ..Default::default()
                })
            }
        }
    }
}
