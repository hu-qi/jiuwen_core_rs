//! 在线演进编排(对齐 `experience/online_orchestrator.py`)。
//!
//! 编排顺序:输入 guard → context 构建 → updater 生成 → OperatorRegistry 本地预览
//! → lifecycle staging → 可选自动审批;底层 store/updater/manager 通过 seam 注入。

use crate::protocols::USER_INTENT_SIGNAL;
use crate::signal::{EvolutionSignal, get_signal_source};

use crate::experience_manager::{build_local_apply_preview, make_pending_change_from_preview};
use crate::experience_query::RecordView;
use crate::experience_types::{
    EvolutionContext, ExperienceApplyResult, ExperienceApprovalRequest, PendingChange,
};
use crate::updates::execute_updates_with_registry;
use ah_contracts::evolving::UpdateKey;
use ah_contracts::keys::{ONLINE_EVOLUTION, OPERATOR};
use ah_contracts::operator::OperatorRegistry;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 在线演进的技能/ledger 读取端口。
pub trait OnlineEvolutionStore: Send + Sync {
    fn skill_exists(&self, skill_name: &str) -> bool;
    fn skill_definition_exists(&self, skill_name: &str) -> bool;
    fn skill_content(&self, skill_name: &str) -> Result<String, String>;
    fn pending_records(&self, skill_name: &str, target: &str) -> Result<Vec<RecordView>, String>;
}

/// 在线 updater 端口;输出未经归一化的结构化更新。
pub trait OnlineEvolutionUpdater: Send + Sync {
    fn generate(
        &self,
        context: &EvolutionContext,
    ) -> Result<BTreeMap<UpdateKey, Option<Value>>, String>;
}

/// 在线生命周期端口;负责 staging、审批与最终持久化。
pub trait OnlineEvolutionManager: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn stage(
        &self,
        pending: PendingChange,
        requires_approval: bool,
        source: &str,
        request_id_prefix: Option<&str>,
        user_query: Option<&str>,
        signal_type: Option<&str>,
        signal_source: Option<&str>,
        messages: Option<Vec<Value>>,
    ) -> Result<ExperienceApprovalRequest, String>;

    fn approve(&self, request_id: &str) -> Result<ExperienceApplyResult, String>;
}

/// 在线演进结果。
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineEvolutionResult {
    pub skill_name: String,
    pub outcome: EvolveOutcome,
    pub request: Option<ExperienceApprovalRequest>,
    pub message: String,
}

/// 真实在线演进编排器:guard → context → updater → local preview → stage/approve。
pub struct OnlineEvolutionOrchestrator {
    store: Arc<dyn OnlineEvolutionStore>,
    updater: Arc<dyn OnlineEvolutionUpdater>,
    manager: Arc<dyn OnlineEvolutionManager>,
    registry: Arc<dyn OperatorRegistry>,
}

impl OnlineEvolutionOrchestrator {
    pub fn new(
        store: Arc<dyn OnlineEvolutionStore>,
        updater: Arc<dyn OnlineEvolutionUpdater>,
        manager: Arc<dyn OnlineEvolutionManager>,
        registry: Arc<dyn OperatorRegistry>,
    ) -> Self {
        Self {
            store,
            updater,
            manager,
            registry,
        }
    }

    pub fn evolve(
        &self,
        skill_name: &str,
        signals: Vec<EvolutionSignal>,
        requires_approval: bool,
        user_query: &str,
        messages: Option<Vec<Value>>,
    ) -> OnlineEvolutionResult {
        let result = |outcome: EvolveOutcome,
                      request: Option<ExperienceApprovalRequest>,
                      message: &str| OnlineEvolutionResult {
            skill_name: skill_name.to_string(),
            outcome,
            request,
            message: message.to_string(),
        };
        if let Some((status, message)) = evolve_skip_guard(
            skill_name,
            signals.is_empty(),
            self.store.skill_exists(skill_name),
            self.store.skill_definition_exists(skill_name),
        ) {
            let outcome = match status.as_str() {
                "skipped_no_input" => EvolveOutcome::SkippedNoInput,
                "skipped_skill_not_found" => EvolveOutcome::SkippedSkillNotFound,
                "skipped_skill_definition_not_found" => {
                    EvolveOutcome::SkippedSkillDefinitionNotFound
                }
                _ => EvolveOutcome::GenerationFailed(message.clone()),
            };
            return result(outcome, None, &message);
        }

        let skill_content = match self.store.skill_content(skill_name) {
            Ok(content) => content,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        let pending_records = |target| self.store.pending_records(skill_name, target);
        let existing_desc_records = match pending_records("description") {
            Ok(records) => records,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        let existing_body_records = match pending_records("body") {
            Ok(records) => records,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        let existing_script_records = match pending_records("script") {
            Ok(records) => records,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        let context = EvolutionContext {
            skill_name: skill_name.to_string(),
            signals: signals.clone(),
            skill_content,
            messages: messages.clone().unwrap_or_default(),
            existing_desc_records,
            existing_body_records,
            user_query: user_query.to_string(),
            existing_script_records,
            tool_call_chain: String::new(),
            metadata: BTreeMap::new(),
        };
        let updates = match self.updater.generate(&context) {
            Ok(updates) => updates,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        let apply_results = execute_updates_with_registry(self.registry.as_ref(), &updates);
        let apply_values: Vec<Value> =
            match apply_results.iter().map(serde_json::to_value).collect() {
                Ok(values) => values,
                Err(error) => {
                    let message = format!("serialize apply results: {error}");
                    return result(
                        EvolveOutcome::GenerationFailed(message.clone()),
                        None,
                        &message,
                    );
                }
            };
        let preview = match build_local_apply_preview(skill_name, &apply_values) {
            Ok(preview) => preview,
            Err(error) => {
                return result(EvolveOutcome::GenerationFailed(error.clone()), None, &error);
            }
        };
        if preview.records.is_empty() {
            return result(
                EvolveOutcome::NoEvolution,
                None,
                &format!("no applied updates for skill={skill_name}"),
            );
        }
        let pending = make_pending_change_from_preview(&preview, None, None, false);
        let preferred = get_preferred_signal(&signals);
        let signal_source = preferred.and_then(get_signal_source);
        let request = match self.manager.stage(
            pending,
            requires_approval,
            "experience_updater",
            None,
            Some(user_query),
            preferred.map(|signal| signal.signal_type.as_str()),
            signal_source.as_deref(),
            messages,
        ) {
            Ok(request) => request,
            Err(error) => {
                return result(
                    EvolveOutcome::PersistenceFailed(error.clone()),
                    None,
                    &error,
                );
            }
        };
        if requires_approval {
            return result(
                EvolveOutcome::Staged,
                Some(request),
                &format!("evolution request staged for skill={skill_name}"),
            );
        }
        let request_id = request.request_id.as_deref().unwrap_or("");
        match self.manager.approve(request_id) {
            Ok(apply) if apply.ok() => result(
                EvolveOutcome::AutoApproved,
                Some(request),
                &format!("evolution request auto-approved for skill={skill_name}"),
            ),
            Ok(apply) => {
                let message = if apply.errors.is_empty() {
                    "persistence failed".to_string()
                } else {
                    apply.errors.join("; ")
                };
                result(
                    EvolveOutcome::PersistenceFailed(message.clone()),
                    Some(request),
                    &message,
                )
            }
            Err(error) => result(
                EvolveOutcome::PersistenceFailed(error.clone()),
                Some(request),
                &error,
            ),
        }
    }
}
/// 在线演进公开 runtime seam。
pub trait OnlineEvolutionRuntime: Seam {
    fn evolve(
        &self,
        skill_name: &str,
        signals: Vec<EvolutionSignal>,
        requires_approval: bool,
        user_query: &str,
        messages: Option<Vec<Value>>,
    ) -> OnlineEvolutionResult;
}

impl Seam for OnlineEvolutionOrchestrator {}

impl OnlineEvolutionRuntime for OnlineEvolutionOrchestrator {
    fn evolve(
        &self,
        skill_name: &str,
        signals: Vec<EvolutionSignal>,
        requires_approval: bool,
        user_query: &str,
        messages: Option<Vec<Value>>,
    ) -> OnlineEvolutionResult {
        Self::evolve(
            self,
            skill_name,
            signals,
            requires_approval,
            user_query,
            messages,
        )
    }
}

/// 确定性在线 updater:将首个有效信号转换为 body experience 预览。
pub struct SignalOnlineUpdater;

impl OnlineEvolutionUpdater for SignalOnlineUpdater {
    fn generate(
        &self,
        context: &EvolutionContext,
    ) -> Result<BTreeMap<UpdateKey, Option<Value>>, String> {
        let signal = get_preferred_signal(&context.signals)
            .ok_or_else(|| "online updater received no signals".to_string())?;
        let content = signal.excerpt.trim();
        if content.is_empty() {
            return Err("online updater received an empty signal excerpt".to_string());
        }
        let section = if crate::protocols::VALID_SECTIONS.contains(&signal.section.as_str()) {
            signal.section.clone()
        } else {
            "Troubleshooting".to_string()
        };
        let record = serde_json::json!({
            "id": format!("signal-{}", crate::tool_metadata::unique_hex()),
            "summary": signal.signal_type,
            "timestamp": crate::experience_types::utc_iso_now(),
            "change": {
                "target": "body",
                "section": section,
                "content": content,
                "action": "append"
            }
        });
        Ok(BTreeMap::from([(
            (
                format!("skill_experience_{}", context.skill_name),
                "experiences".to_string(),
            ),
            Some(Value::Array(vec![record])),
        )]))
    }
}

/// 生产在线演进插件:文件 store + 确定性 updater + operator registry。
pub struct OnlineEvolutionPlugin {
    skills_root: std::path::PathBuf,
}

impl OnlineEvolutionPlugin {
    pub fn new(skills_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            skills_root: skills_root.into(),
        }
    }
}

impl Plugin for OnlineEvolutionPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-online-evolution"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ONLINE_EVOLUTION]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![OPERATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "operator seam not registered".to_string(),
            })?;
        std::fs::create_dir_all(&self.skills_root).map_err(|error| PluginError::Apply {
            plugin: self.name(),
            message: format!("create skills root: {error}"),
        })?;
        let mut effects = Vec::new();
        for entry in std::fs::read_dir(&self.skills_root).map_err(|error| PluginError::Apply {
            plugin: self.name(),
            message: format!("read skills root: {error}"),
        })? {
            let entry = entry.map_err(|error| PluginError::Apply {
                plugin: self.name(),
                message: format!("read skill entry: {error}"),
            })?;
            if entry.file_name() == ".pending" {
                continue;
            }
            if entry
                .file_type()
                .map_err(|error| PluginError::Apply {
                    plugin: self.name(),
                    message: format!("read skill entry type: {error}"),
                })?
                .is_dir()
            {
                let skill_name = entry.file_name().to_string_lossy().into_owned();
                effects.push(registry.register(Arc::new(
                    crate::online_file::FileSkillExperienceOperator::new(skill_name),
                )));
            }
        }
        let store = Arc::new(crate::online_file::FileEvolutionStore::new(
            self.skills_root.clone(),
        ));
        let manager = Arc::new(
            crate::online_file::FileEvolutionManager::open(store.clone()).map_err(|message| {
                PluginError::Apply {
                    plugin: self.name(),
                    message,
                }
            })?,
        );
        let runtime: Arc<dyn OnlineEvolutionRuntime> = Arc::new(OnlineEvolutionOrchestrator::new(
            store,
            Arc::new(SignalOnlineUpdater),
            manager,
            registry,
        ));
        effects.push(ctx.register(ONLINE_EVOLUTION, runtime));
        Ok(effects)
    }
}
/// 优先信号(对齐 _get_preferred_signal:首个 user_intent 信号,否则首个信号,空 → None)。
pub fn get_preferred_signal(signals: &[EvolutionSignal]) -> Option<&EvolutionSignal> {
    signals
        .iter()
        .find(|s| s.signal_type == USER_INTENT_SIGNAL)
        .or_else(|| signals.first())
}

/// 信号类型(对齐 _get_signal_type)。
pub fn get_signal_type(signals: &[EvolutionSignal]) -> Option<String> {
    get_preferred_signal(signals).map(|s| s.signal_type.clone())
}

/// 信号来源(对齐 _get_signal_source)。
pub fn get_signal_source_of(signals: &[EvolutionSignal]) -> Option<String> {
    get_preferred_signal(signals).and_then(get_signal_source)
}

/// evolve 状态决策守卫(对齐 evolve() 的开头守卫;返回跳过原因消息或 None 继续)。
pub fn evolve_skip_guard(
    skill_name: &str,
    signals_empty: bool,
    skill_exists: bool,
    skill_definition_exists: bool,
) -> Option<(String, String)> {
    // 返回 (status, message)
    if skill_name.is_empty() || signals_empty {
        return Some((
            "skipped_no_input".to_string(),
            "online evolution skipped because skill_name or signals are empty".to_string(),
        ));
    }
    if !skill_exists {
        return Some((
            "skipped_skill_not_found".to_string(),
            format!("online evolution skipped because skill '{skill_name}' does not exist"),
        ));
    }
    if !skill_definition_exists {
        return Some((
            "skipped_skill_definition_not_found".to_string(),
            format!("online evolution skipped because skill '{skill_name}' is missing SKILL.md"),
        ));
    }
    None
}

/// 结果阶段决策(对齐 evolve() 的后置分支)。
#[derive(Debug, Clone, PartialEq)]
pub enum EvolveOutcome {
    /// 输入为空(skipped_no_input)。
    SkippedNoInput,
    /// 技能不存在(skipped_skill_not_found)。
    SkippedSkillNotFound,
    /// 缺少 SKILL.md(skipped_skill_definition_not_found)。
    SkippedSkillDefinitionNotFound,
    /// 生成失败(generation_failed)。
    GenerationFailed(String),
    /// 无演进记录(no_evolution_no_records)。
    NoEvolution,
    /// 需要审批:staged。
    Staged,
    /// 自动批准成功:auto_approved。
    AutoApproved,
    /// 持久化失败:persistence_failed。
    PersistenceFailed(String),
}

/// 根据生成预览与审批结果推导最终阶段(对齐 evolve() 后置逻辑)。
pub fn decide_evolve_outcome(
    preview_records_empty: bool,
    requires_approval: bool,
    apply_ok: bool,
    apply_errors: &[String],
    generation_error: Option<&str>,
) -> EvolveOutcome {
    if let Some(err) = generation_error {
        return EvolveOutcome::GenerationFailed(err.to_string());
    }
    if preview_records_empty {
        return EvolveOutcome::NoEvolution;
    }
    if requires_approval {
        return EvolveOutcome::Staged;
    }
    if !apply_ok {
        let message = if apply_errors.is_empty() {
            "persistence failed".to_string()
        } else {
            apply_errors.join("; ")
        };
        return EvolveOutcome::PersistenceFailed(message);
    }
    EvolveOutcome::AutoApproved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::make_evolution_signal;

    fn sig(t: &str, source: &str) -> EvolutionSignal {
        make_evolution_signal(t, "S", "excerpt", None, None, Some(source), None)
    }

    #[test]
    fn preferred_signal_prefers_user_intent() {
        let signals = vec![
            sig("execution_failure", "a"),
            sig("user_intent", "b"),
            sig("low_score", "c"),
        ];
        let preferred = get_preferred_signal(&signals).unwrap();
        assert_eq!(preferred.signal_type, "user_intent");
        assert_eq!(get_signal_type(&signals).as_deref(), Some("user_intent"));
        assert_eq!(get_signal_source_of(&signals).as_deref(), Some("b"));
    }

    #[test]
    fn preferred_signal_falls_back_to_first() {
        let signals = vec![sig("low_score", "x"), sig("evaluated", "y")];
        assert_eq!(get_signal_type(&signals).as_deref(), Some("low_score"));
        assert_eq!(get_signal_source_of(&signals).as_deref(), Some("x"));
    }

    #[test]
    fn preferred_signal_empty() {
        assert_eq!(get_preferred_signal(&[]), None);
        assert_eq!(get_signal_type(&[]), None);
        assert_eq!(get_signal_source_of(&[]), None);
    }

    #[test]
    fn skip_guards() {
        let no_input = evolve_skip_guard("", false, true, true).unwrap();
        assert_eq!(no_input.0, "skipped_no_input");
        let no_signals = evolve_skip_guard("sk", true, true, true).unwrap();
        assert_eq!(no_signals.0, "skipped_no_input");
        let no_skill = evolve_skip_guard("sk", false, false, true).unwrap();
        assert_eq!(no_skill.0, "skipped_skill_not_found");
        let no_def = evolve_skip_guard("sk", false, true, false).unwrap();
        assert_eq!(no_def.0, "skipped_skill_definition_not_found");
        assert_eq!(evolve_skip_guard("sk", false, true, true), None);
    }

    #[test]
    fn outcome_decisions() {
        assert_eq!(
            decide_evolve_outcome(false, true, false, &[], Some("boom")),
            EvolveOutcome::GenerationFailed("boom".to_string()),
        );
        assert_eq!(
            decide_evolve_outcome(true, false, false, &[], None),
            EvolveOutcome::NoEvolution,
        );
        assert_eq!(
            decide_evolve_outcome(false, true, false, &[], None),
            EvolveOutcome::Staged,
        );
        assert_eq!(
            decide_evolve_outcome(false, false, true, &[], None),
            EvolveOutcome::AutoApproved,
        );
        assert_eq!(
            decide_evolve_outcome(
                false,
                false,
                false,
                &["e1".to_string(), "e2".to_string()],
                None
            ),
            EvolveOutcome::PersistenceFailed("e1; e2".to_string()),
        );
        assert_eq!(
            decide_evolve_outcome(false, false, false, &[], None),
            EvolveOutcome::PersistenceFailed("persistence failed".to_string()),
        );
    }
    #[test]
    fn orchestrator_runs_update_preview_and_stages_request() {
        use crate::experience_types::{ExperienceApplyResult, ExperienceProposal};
        use ah_contracts::evolving::UpdateKey;
        use ah_contracts::keys::OPERATOR;
        use ah_contracts::operator::OperatorRegistry;
        use ah_hub::context::Context;
        use ah_hub::plugin::DynPlugin;
        use serde_json::{Value, json};
        use std::collections::BTreeMap;
        use std::sync::Arc;

        struct Store;
        impl OnlineEvolutionStore for Store {
            fn skill_exists(&self, _: &str) -> bool {
                true
            }
            fn skill_definition_exists(&self, _: &str) -> bool {
                true
            }
            fn skill_content(&self, _: &str) -> Result<String, String> {
                Ok("skill body".into())
            }
            fn pending_records(&self, _: &str, _: &str) -> Result<Vec<RecordView>, String> {
                Ok(vec![])
            }
        }
        struct Updater;
        impl OnlineEvolutionUpdater for Updater {
            fn generate(
                &self,
                _: &EvolutionContext,
            ) -> Result<BTreeMap<UpdateKey, Option<Value>>, String> {
                Ok(BTreeMap::from([(
                    ("skill_experience_sk".into(), "experiences".into()),
                    Some(
                        json!([{"id": "ev-1", "summary": "checks", "timestamp": "now", "change": {"target": "body", "section": "Troubleshooting", "content": "use checks"}}]),
                    ),
                )]))
            }
        }
        struct Manager;
        impl OnlineEvolutionManager for Manager {
            fn stage(
                &self,
                pending: PendingChange,
                _: bool,
                _: &str,
                _: Option<&str>,
                _: Option<&str>,
                _: Option<&str>,
                _: Option<&str>,
                _: Option<Vec<Value>>,
            ) -> Result<ExperienceApprovalRequest, String> {
                Ok(ExperienceApprovalRequest {
                    skill_name: pending.skill_name.clone(),
                    proposal: ExperienceProposal {
                        skill_name: pending.skill_name.clone(),
                        records: pending.payload.clone(),
                        requires_approval: true,
                        source: "test".into(),
                        user_query: String::new(),
                        signal_type: None,
                        signal_source: None,
                    },
                    pending_change: Some(pending),
                    request_id: Some("req-1".into()),
                })
            }
            fn approve(&self, _: &str) -> Result<ExperienceApplyResult, String> {
                Ok(ExperienceApplyResult {
                    skill_name: "sk".into(),
                    applied_count: 1,
                    ..Default::default()
                })
            }
        }

        let ctx = Context::new();
        let mut effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_operator::OperatorPlugin) as DynPlugin
            ])
            .expect("mount operator");
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        effects.push(registry.register(Arc::new(
            ah_plugins_operator::SkillExperienceOperator::new("sk"),
        )));
        let orchestrator = OnlineEvolutionOrchestrator::new(
            Arc::new(Store),
            Arc::new(Updater),
            Arc::new(Manager),
            registry,
        );
        let result = orchestrator.evolve(
            "sk",
            vec![sig("user_intent", "operator")],
            false,
            "query",
            None,
        );
        assert_eq!(result.outcome, EvolveOutcome::AutoApproved);
        assert_eq!(result.request.unwrap().request_id.as_deref(), Some("req-1"));
        drop(effects);
    }
    #[test]
    fn file_evolution_manager_persists_log_and_skill_atomically() {
        use crate::experience_query::RecordView;
        use crate::experience_types::PendingChange;
        use crate::online_file::{FileEvolutionManager, FileEvolutionStore};
        use std::fs;
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("ah-online-file-{}", std::process::id()));
        let skill_dir = root.join("sk");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sk\ndescription: base\n---\n\n# Skill\n",
        )
        .unwrap();

        let store = Arc::new(FileEvolutionStore::new(&root));
        let manager = FileEvolutionManager::open(store).unwrap();
        let pending = PendingChange::make(
            "sk",
            &[RecordView {
                id: "preview-1".into(),
                summary: Some("checks".into()),
                score: 0.8,
                timestamp: "now".into(),
                target: "body".into(),
                section: "Troubleshooting".into(),
                content: "Run checks before publishing.".into(),
            }],
            None,
            None,
        );
        let request = manager
            .stage(
                pending,
                false,
                "experience_updater",
                None,
                Some("query"),
                Some("user_intent"),
                Some("operator"),
                None,
            )
            .unwrap();
        let applied = manager
            .approve(request.request_id.as_deref().unwrap())
            .unwrap();
        assert_eq!(applied.applied_count, 1);
        assert!(applied.errors.is_empty());

        let skill = fs::read_to_string(skill_dir.join("SKILL.md")).unwrap();
        assert!(skill.contains("Run checks before publishing."));
        let log: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(skill_dir.join("evolutions.json")).unwrap())
                .unwrap();
        assert_eq!(log["entries"].as_array().unwrap().len(), 1);
        assert_eq!(log["entries"][0]["applied"], true);
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn online_evolution_plugin_runs_file_backed_pipeline() {
        use ah_contracts::keys::ONLINE_EVOLUTION;
        use ah_hub::context::Context;
        use ah_hub::plugin::DynPlugin;
        use std::fs;
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("ah-online-plugin-{}", std::process::id()));
        let skill_dir = root.join("sk");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::create_dir_all(root.join(".pending")).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sk\ndescription: base\n---\n",
        )
        .unwrap();
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_operator::OperatorPlugin) as DynPlugin,
                Arc::new(OnlineEvolutionPlugin::new(&root)) as DynPlugin,
            ])
            .expect("mount online evolution");
        let operator_registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("operator registry");
        assert!(
            operator_registry
                .ids()
                .iter()
                .all(|id| id != "skill_experience_.pending")
        );
        let runtime = ctx
            .service::<dyn OnlineEvolutionRuntime>(&ONLINE_EVOLUTION)
            .expect("online evolution runtime");
        let result = runtime.evolve(
            "sk",
            vec![sig("user_intent", "operator")],
            false,
            "query",
            None,
        );
        assert_eq!(result.outcome, EvolveOutcome::AutoApproved);
        assert!(
            fs::read_to_string(skill_dir.join("SKILL.md"))
                .unwrap()
                .contains("excerpt")
        );
        drop(effects);
        let _ = fs::remove_dir_all(&root);
    }
    #[test]
    fn file_store_renders_script_target_and_index_link() {
        use crate::experience_query::RecordView;
        use crate::online_file::FileEvolutionStore;
        use std::fs;

        let root = std::env::temp_dir().join(format!("ah-online-script-{}", std::process::id()));
        let skill_dir = root.join("sk");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sk\ndescription: base\n---\n",
        )
        .unwrap();
        let store = FileEvolutionStore::new(&root);
        store
            .append_records(
                "sk",
                &[RecordView {
                    id: "script-1".into(),
                    summary: Some("validation script".into()),
                    score: 0.9,
                    timestamp: "now".into(),
                    target: "script".into(),
                    section: "Scripts".into(),
                    content: "echo validated".into(),
                }],
                "test",
            )
            .unwrap();
        assert_eq!(
            fs::read_to_string(skill_dir.join("evolution/scripts/script-1.md")).unwrap(),
            "echo validated"
        );
        assert!(
            fs::read_to_string(skill_dir.join("SKILL.md"))
                .unwrap()
                .contains("evolution/scripts/script-1.md")
        );
        let log: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(skill_dir.join("evolutions.json")).unwrap())
                .unwrap();
        assert_eq!(
            log["entries"][0]["change"]["script_filename"],
            "script-1.md"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_manager_restores_pending_change_after_reopen() {
        use crate::experience_query::RecordView;
        use crate::experience_types::PendingChange;
        use crate::online_file::{FileEvolutionManager, FileEvolutionStore};
        use std::fs;
        use std::sync::Arc;

        let root = std::env::temp_dir().join(format!("ah-online-pending-{}", std::process::id()));
        let skill_dir = root.join("sk");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: sk\ndescription: base\n---\n",
        )
        .unwrap();
        let store = Arc::new(FileEvolutionStore::new(&root));
        let manager = FileEvolutionManager::open(store.clone()).unwrap();
        let pending = PendingChange::make(
            "sk",
            &[RecordView {
                id: "pending-1".into(),
                summary: Some("resume".into()),
                score: 0.7,
                timestamp: "now".into(),
                target: "body".into(),
                section: "Troubleshooting".into(),
                content: "Persist approvals across restart.".into(),
            }],
            None,
            None,
        );
        let request = manager
            .stage(pending, true, "test", None, None, None, None, None)
            .unwrap();
        let request_id = request.request_id.unwrap();
        drop(manager);

        let reopened = FileEvolutionManager::open(store).unwrap();
        let applied = reopened.approve(&request_id).unwrap();
        assert_eq!(applied.applied_count, 1);
        assert!(
            !root
                .join(".pending")
                .join(format!("{request_id}.json"))
                .exists()
        );
        assert!(
            fs::read_to_string(skill_dir.join("SKILL.md"))
                .unwrap()
                .contains("Persist approvals")
        );
        let _ = fs::remove_dir_all(&root);
    }
}
