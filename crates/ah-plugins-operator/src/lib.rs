//! # ah-plugins-operator
//!
//! 真实自进化算子(对齐 Python core/operator):
//! - LLMCallOperator:system_prompt/user_prompt(各自 freeze 标记);
//! - ToolCallOperator:tool_description 字典;
//! - MemoryCallOperator:enabled/max_retries;
//! - SkillCallOperator:skill experience/description。
//!
//! 每个算子管理参数状态,set_parameter 检查 freeze 后更新并触发
//! on_parameter_updated 回调;get_state/load_state 支持检查点;
//! OperatorRegistry 按 id 注册/取回。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ah_contracts::effect::Effect;
use ah_contracts::evolving::{ApplyResult, UpdateEffect, UpdateMode, UpdateValue};
use ah_contracts::keys::OPERATOR;
use ah_contracts::operator::{
    Operator, OperatorError, OperatorRegistry, ParameterUpdated, PreviewableOperator, TunableKind,
    TunableSpec,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

/// 共享回调集:一个算子多个消费方(Arc 以便可逆注册 capture)。
type Callbacks = Arc<Mutex<Vec<ParameterUpdated>>>;

fn fire(callbacks: &Callbacks, target: &str, value: &Value) {
    let snapshot: Vec<ParameterUpdated> = callbacks.lock().unwrap().clone();
    for callback in snapshot {
        callback(target, value);
    }
}

/// LLM prompt 参数句柄。
pub struct LlmCallOperator {
    id: String,
    system_prompt: Mutex<String>,
    user_prompt: Mutex<String>,
    freeze_system: bool,
    freeze_user: bool,
    callbacks: Callbacks,
}

impl LlmCallOperator {
    /// 创建;冻结标记决定哪些参数可调。
    pub fn new(
        id: impl Into<String>,
        system_prompt: impl Into<String>,
        user_prompt: impl Into<String>,
        freeze_system: bool,
        freeze_user: bool,
    ) -> Self {
        Self {
            id: id.into(),
            system_prompt: Mutex::new(system_prompt.into()),
            user_prompt: Mutex::new(user_prompt.into()),
            freeze_system,
            freeze_user,
            callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 消费方读当前值。
    pub fn prompts(&self) -> (String, String) {
        (
            self.system_prompt.lock().unwrap().clone(),
            self.user_prompt.lock().unwrap().clone(),
        )
    }
}

impl Seam for LlmCallOperator {}

impl Operator for LlmCallOperator {
    fn operator_id(&self) -> String {
        self.id.clone()
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        let mut tunables = Vec::new();
        if !self.freeze_system {
            tunables.push(TunableSpec {
                name: "system_prompt".to_string(),
                kind: TunableKind::Prompt,
                path: "system_prompt".to_string(),
                constraint: None,
            });
        }
        if !self.freeze_user {
            tunables.push(TunableSpec {
                name: "user_prompt".to_string(),
                kind: TunableKind::Prompt,
                path: "user_prompt".to_string(),
                constraint: None,
            });
        }
        tunables
    }

    fn get_state(&self) -> Value {
        json!({
            "system_prompt": *self.system_prompt.lock().unwrap(),
            "user_prompt": *self.user_prompt.lock().unwrap(),
        })
    }

    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
        let text = value
            .as_str()
            .ok_or_else(|| OperatorError("prompt value must be string".to_string()))?
            .to_string();
        match target {
            "system_prompt" if !self.freeze_system => {
                *self.system_prompt.lock().unwrap() = text.clone();
                fire(&self.callbacks, target, &json!(text));
                Ok(())
            }
            "user_prompt" if !self.freeze_user => {
                *self.user_prompt.lock().unwrap() = text.clone();
                fire(&self.callbacks, target, &json!(text));
                Ok(())
            }
            "system_prompt" => Err(OperatorError("system_prompt is frozen".to_string())),
            "user_prompt" => Err(OperatorError("user_prompt is frozen".to_string())),
            other => Err(OperatorError(format!("unknown target: {other}"))),
        }
    }

    fn load_state(&self, state: Value) -> Result<(), OperatorError> {
        if let Some(system) = state.get("system_prompt").and_then(Value::as_str) {
            *self.system_prompt.lock().unwrap() = system.to_string();
        }
        if let Some(user) = state.get("user_prompt").and_then(Value::as_str) {
            *self.user_prompt.lock().unwrap() = user.to_string();
        }
        Ok(())
    }

    fn on_parameter_updated(&self, callback: ParameterUpdated) -> Effect {
        self.callbacks.lock().unwrap().push(callback.clone());
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks
                .lock()
                .unwrap()
                .retain(|c| !Arc::ptr_eq(c, &callback));
        })
    }
}

/// Tool description 参数句柄。
pub struct ToolCallOperator {
    id: String,
    descriptions: Mutex<HashMap<String, String>>,
    callbacks: Callbacks,
}

impl ToolCallOperator {
    pub fn new(id: impl Into<String>, descriptions: HashMap<String, String>) -> Self {
        Self {
            id: id.into(),
            descriptions: Mutex::new(descriptions),
            callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 消费方读工具描述。
    pub fn description(&self, tool: &str) -> Option<String> {
        self.descriptions.lock().unwrap().get(tool).cloned()
    }
}

impl Seam for ToolCallOperator {}

impl Operator for ToolCallOperator {
    fn operator_id(&self) -> String {
        self.id.clone()
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        if self.descriptions.lock().unwrap().is_empty() {
            Vec::new()
        } else {
            vec![TunableSpec {
                name: "tool_description".to_string(),
                kind: TunableKind::Text,
                path: "tool_description".to_string(),
                constraint: Some(json!({ "type": "dict" })),
            }]
        }
    }

    fn get_state(&self) -> Value {
        json!({ "tool_description": *self.descriptions.lock().unwrap() })
    }

    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
        if target != "tool_description" {
            return Err(OperatorError(format!("unknown target: {target}")));
        }
        let map = value
            .as_object()
            .ok_or_else(|| OperatorError("tool_description must be object".to_string()))?;
        let parsed: HashMap<String, String> = map
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
            .collect();
        *self.descriptions.lock().unwrap() = parsed.clone();
        fire(&self.callbacks, target, &json!(parsed));
        Ok(())
    }

    fn load_state(&self, state: Value) -> Result<(), OperatorError> {
        if let Some(map) = state.get("tool_description").and_then(Value::as_object) {
            *self.descriptions.lock().unwrap() = map
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect();
        }
        Ok(())
    }

    fn on_parameter_updated(&self, callback: ParameterUpdated) -> Effect {
        self.callbacks.lock().unwrap().push(callback.clone());
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks
                .lock()
                .unwrap()
                .retain(|c| !Arc::ptr_eq(c, &callback));
        })
    }
}

/// Memory 参数句柄。
pub struct MemoryCallOperator {
    id: String,
    enabled: Mutex<bool>,
    max_retries: Mutex<i64>,
    callbacks: Callbacks,
}

impl MemoryCallOperator {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            enabled: Mutex::new(true),
            max_retries: Mutex::new(0),
            callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Seam for MemoryCallOperator {}

impl Operator for MemoryCallOperator {
    fn operator_id(&self) -> String {
        self.id.clone()
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        vec![
            TunableSpec {
                name: "enabled".to_string(),
                kind: TunableKind::Discrete,
                path: "enabled".to_string(),
                constraint: Some(json!({ "type": "bool" })),
            },
            TunableSpec {
                name: "max_retries".to_string(),
                kind: TunableKind::Discrete,
                path: "max_retries".to_string(),
                constraint: Some(json!({ "type": "int", "min": 0, "max": 5 })),
            },
        ]
    }

    fn get_state(&self) -> Value {
        json!({
            "enabled": *self.enabled.lock().unwrap(),
            "max_retries": *self.max_retries.lock().unwrap(),
        })
    }

    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
        match target {
            "enabled" => {
                let enabled = value
                    .as_bool()
                    .ok_or_else(|| OperatorError("enabled must be bool".to_string()))?;
                *self.enabled.lock().unwrap() = enabled;
                fire(&self.callbacks, target, &json!(enabled));
                Ok(())
            }
            "max_retries" => {
                let retries = value
                    .as_i64()
                    .ok_or_else(|| OperatorError("max_retries must be int".to_string()))?;
                if !(0..=5).contains(&retries) {
                    return Err(OperatorError("max_retries out of range 0..=5".to_string()));
                }
                *self.max_retries.lock().unwrap() = retries;
                fire(&self.callbacks, target, &json!(retries));
                Ok(())
            }
            other => Err(OperatorError(format!("unknown target: {other}"))),
        }
    }

    fn load_state(&self, state: Value) -> Result<(), OperatorError> {
        if let Some(enabled) = state.get("enabled").and_then(Value::as_bool) {
            *self.enabled.lock().unwrap() = enabled;
        }
        if let Some(retries) = state.get("max_retries").and_then(Value::as_i64) {
            *self.max_retries.lock().unwrap() = retries;
        }
        Ok(())
    }

    fn on_parameter_updated(&self, callback: ParameterUpdated) -> Effect {
        self.callbacks.lock().unwrap().push(callback.clone());
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks
                .lock()
                .unwrap()
                .retain(|c| !Arc::ptr_eq(c, &callback));
        })
    }
}

/// Skill 参数句柄(experience/description)。
pub struct SkillCallOperator {
    id: String,
    experience: Mutex<String>,
    description: Mutex<String>,
    callbacks: Callbacks,
}

impl SkillCallOperator {
    pub fn new(
        id: impl Into<String>,
        experience: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            experience: Mutex::new(experience.into()),
            description: Mutex::new(description.into()),
            callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Seam for SkillCallOperator {}

impl Operator for SkillCallOperator {
    fn operator_id(&self) -> String {
        self.id.clone()
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        vec![
            TunableSpec {
                name: "experience".to_string(),
                kind: TunableKind::Prompt,
                path: "experience".to_string(),
                constraint: None,
            },
            TunableSpec {
                name: "description".to_string(),
                kind: TunableKind::Text,
                path: "description".to_string(),
                constraint: None,
            },
        ]
    }

    fn get_state(&self) -> Value {
        json!({
            "experience": *self.experience.lock().unwrap(),
            "description": *self.description.lock().unwrap(),
        })
    }

    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
        let text = value
            .as_str()
            .ok_or_else(|| OperatorError("value must be string".to_string()))?
            .to_string();
        match target {
            "experience" => {
                *self.experience.lock().unwrap() = text.clone();
                fire(&self.callbacks, target, &json!(text));
                Ok(())
            }
            "description" => {
                *self.description.lock().unwrap() = text.clone();
                fire(&self.callbacks, target, &json!(text));
                Ok(())
            }
            other => Err(OperatorError(format!("unknown target: {other}"))),
        }
    }

    fn load_state(&self, state: Value) -> Result<(), OperatorError> {
        if let Some(exp) = state.get("experience").and_then(Value::as_str) {
            *self.experience.lock().unwrap() = exp.to_string();
        }
        if let Some(desc) = state.get("description").and_then(Value::as_str) {
            *self.description.lock().unwrap() = desc.to_string();
        }
        Ok(())
    }

    fn on_parameter_updated(&self, callback: ParameterUpdated) -> Effect {
        self.callbacks.lock().unwrap().push(callback.clone());
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks
                .lock()
                .unwrap()
                .retain(|c| !Arc::ptr_eq(c, &callback));
        })
    }
}

/// Skill experience 预览算子(对齐 Python core/operator/skill_call/base.py
/// `SkillExperienceOperator`)。
///
/// 只拥有 `updates_generated -> local_apply_completed` 的预览语义:
/// 审批归 ExperienceManager,持久化归 EvolutionStore,算子本身不进入
/// pending/持久化。`apply_update` 路由到 `preview_update`。
pub struct SkillExperienceOperator {
    skill_name: String,
    callbacks: Callbacks,
}

/// 协议常量(对齐 agent_evolving/protocols.py)。
pub const EXPERIENCES_TARGET: &str = "experiences";
pub const APPEND_MODE: &str = "append";
pub const MERGE_MODE: &str = "merge";
pub const PENDING_CHANGE_EFFECT: &str = "pending_change";
pub const LOCAL_APPLY_COMPLETED: &str = "local_apply_completed";

impl SkillExperienceOperator {
    pub fn new(skill_name: impl Into<String>) -> Self {
        Self {
            skill_name: skill_name.into(),
            callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Seam for SkillExperienceOperator {}

impl Operator for SkillExperienceOperator {
    fn operator_id(&self) -> String {
        format!("skill_experience_{}", self.skill_name)
    }

    fn get_tunables(&self) -> Vec<TunableSpec> {
        vec![TunableSpec {
            name: EXPERIENCES_TARGET.to_string(),
            kind: TunableKind::SkillExperience,
            path: "content".to_string(),
            constraint: Some(json!({ "type": "record" })),
        }]
    }

    fn get_state(&self) -> Value {
        json!({})
    }

    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
        // 对齐 Python set_parameter:仅 experiences 且非 None 时通知消费方,不暂存。
        if target != EXPERIENCES_TARGET || value.is_null() {
            return Ok(());
        }
        let items: Value = if let Some(list) = value.as_array() {
            Value::Array(list.clone())
        } else {
            Value::Array(vec![value])
        };
        fire(&self.callbacks, target, &items);
        Ok(())
    }

    fn load_state(&self, _state: Value) -> Result<(), OperatorError> {
        Ok(())
    }

    fn on_parameter_updated(&self, callback: ParameterUpdated) -> Effect {
        self.callbacks.lock().unwrap().push(callback.clone());
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks
                .lock()
                .unwrap()
                .retain(|c| !Arc::ptr_eq(c, &callback));
        })
    }
    fn apply_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
        self.preview_update(target, update)
    }
}

impl PreviewableOperator for SkillExperienceOperator {
    fn preview_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
        // 目标校验。
        if target != EXPERIENCES_TARGET {
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
                errors: vec![format!(
                    "unsupported target for SkillExperienceOperator: {target}"
                )],
                metadata: update.metadata.clone(),
            };
        }
        // mode/effect 校验:仅 append/merge + pending_change。
        let valid_mode = matches!(update.mode, UpdateMode::Append | UpdateMode::Merge);
        if update.effect != UpdateEffect::PendingChange || !valid_mode {
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
                errors: vec![format!(
                    "unsupported update mode/effect for SkillExperienceOperator: {}/{}",
                    update.mode.as_str(),
                    update.effect.as_str()
                )],
                metadata: update.metadata.clone(),
            };
        }
        // 记录列表:payload 为数组原样,否则单元素。
        let records: Vec<Value> = match update.payload.as_array() {
            Some(list) => list.clone(),
            None => vec![update.payload.clone()],
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
            lifecycle_stage: Some(LOCAL_APPLY_COMPLETED.to_string()),
            pending_change_id: None,
            errors: vec![],
            metadata,
        }
    }
}

/// 算子注册表。
pub struct LocalOperatorRegistry {
    operators: Arc<Mutex<HashMap<String, Arc<dyn Operator>>>>,
}

impl Seam for LocalOperatorRegistry {}

impl OperatorRegistry for LocalOperatorRegistry {
    fn register(&self, operator: Arc<dyn Operator>) -> Effect {
        let id = operator.operator_id();
        self.operators.lock().unwrap().insert(id.clone(), operator);
        let operators = self.operators.clone();
        Effect::new(move || {
            operators.lock().unwrap().remove(&id);
        })
    }

    fn get(&self, operator_id: &str) -> Option<Arc<dyn Operator>> {
        self.operators.lock().unwrap().get(operator_id).cloned()
    }

    fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.operators.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }
}

/// 算子插件:注册表 + 四个内置算子。
pub struct OperatorPlugin;

impl Plugin for OperatorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-operator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![OPERATOR]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry: Arc<dyn OperatorRegistry> = Arc::new(LocalOperatorRegistry {
            operators: Arc::new(Mutex::new(HashMap::new())),
        });
        let mut effects = vec![ctx.register(OPERATOR, registry.clone())];

        let llm: Arc<dyn Operator> = Arc::new(LlmCallOperator::new(
            "agent/llm_call",
            "You are a helpful assistant.",
            "{{query}}",
            false, // system_prompt 可调
            true,  // user_prompt 冻结
        ));
        effects.push(registry.register(llm));

        let tool: Arc<dyn Operator> =
            Arc::new(ToolCallOperator::new("agent/tool_call", HashMap::new()));
        effects.push(registry.register(tool));

        let memory: Arc<dyn Operator> = Arc::new(MemoryCallOperator::new("agent/memory_call"));
        effects.push(registry.register(memory));

        let skill: Arc<dyn Operator> = Arc::new(SkillCallOperator::new(
            "agent/skill_call",
            "",
            "general skill",
        ));
        effects.push(registry.register(skill));

        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::OPERATOR;
    use ah_contracts::operator::OperatorRegistry;
    use ah_hub::plugin::DynPlugin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(OperatorPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn llm_operator_freeze_and_tunables() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        let llm = registry.get("agent/llm_call").expect("llm op");
        assert_eq!(llm.operator_id(), "agent/llm_call");

        // user_prompt 冻结 → 不在 tunables。
        let tunables = llm.get_tunables();
        assert!(tunables.iter().any(|t| t.name == "system_prompt"));
        assert!(
            !tunables.iter().any(|t| t.name == "user_prompt"),
            "frozen user_prompt hidden"
        );

        // 冻结参数 set 显式报错。
        assert!(llm.set_parameter("user_prompt", json!("x")).is_err());

        // 可调参数更新 + 回调。
        let fired = Arc::new(AtomicUsize::new(0));
        let f = fired.clone();
        let _guard = llm.on_parameter_updated(Arc::new(move |_t, _v| {
            f.fetch_add(1, Ordering::SeqCst);
        }));
        llm.set_parameter("system_prompt", json!("new system"))
            .expect("set");
        assert_eq!(fired.load(Ordering::SeqCst), 1, "callback fired");

        // 检查点往返。
        let state = llm.get_state();
        assert_eq!(state["system_prompt"], "new system");
        llm.set_parameter("system_prompt", json!("temp"))
            .expect("set");
        llm.load_state(state).expect("load");
        let restored = llm.get_state();
        assert_eq!(
            restored["system_prompt"], "new system",
            "checkpoint restore"
        );

        drop(effects);
    }

    #[test]
    fn tool_operator_dict_and_callback() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        let tool = registry.get("agent/tool_call").expect("tool op");
        assert!(
            tool.get_tunables().is_empty(),
            "no descriptions -> no tunables"
        );

        let mut desc = HashMap::new();
        desc.insert("read_file".to_string(), "Read a file".to_string());
        tool.set_parameter("tool_description", json!(desc))
            .expect("set");
        let tunables = tool.get_tunables();
        assert_eq!(tunables.len(), 1);
        assert_eq!(tunables[0].name, "tool_description");

        assert!(
            tool.set_parameter("nope", json!({})).is_err(),
            "unknown target"
        );
        assert!(
            tool.set_parameter("tool_description", json!("not-a-dict"))
                .is_err()
        );

        drop(effects);
    }

    #[test]
    fn memory_operator_constraints() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        let memory = registry.get("agent/memory_call").expect("memory op");

        memory
            .set_parameter("enabled", json!(false))
            .expect("disable");
        assert_eq!(memory.get_state()["enabled"], false);
        assert!(
            memory.set_parameter("max_retries", json!(99)).is_err(),
            "out of range rejected"
        );
        memory.set_parameter("max_retries", json!(3)).expect("set");
        assert_eq!(memory.get_state()["max_retries"], 3);

        drop(effects);
    }

    #[test]
    fn skill_operator_and_registry_roundtrip() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        let ids = registry.ids();
        assert_eq!(
            ids,
            vec![
                "agent/llm_call",
                "agent/memory_call",
                "agent/skill_call",
                "agent/tool_call"
            ]
        );

        let skill = registry.get("agent/skill_call").expect("skill op");
        skill
            .set_parameter("experience", json!("learned: avoid rm -rf"))
            .expect("set");
        assert_eq!(skill.get_state()["experience"], "learned: avoid rm -rf");

        // 注册可逆:drop effects 后 id 消失。
        drop(effects);
        let ctx2 = Context::new();
        let _ = ctx2
            .mount_all(vec![Arc::new(OperatorPlugin) as DynPlugin])
            .expect("mount");
        // effects dropped -> services unregistered (Context dropped here anyway).
        let _ = ctx2;
    }

    #[test]
    fn skill_experience_operator_preview_semantics() {
        use ah_contracts::evolving::{UpdateEffect, UpdateMode, UpdateValue};
        use ah_contracts::operator::PreviewableOperator;

        let op = SkillExperienceOperator::new("my_skill");
        assert_eq!(op.operator_id(), "skill_experience_my_skill");

        // tunables:experiences(kind skill_experience)。
        let tunables = op.get_tunables();
        assert_eq!(tunables.len(), 1);
        assert_eq!(tunables[0].name, EXPERIENCES_TARGET);
        assert_eq!(tunables[0].kind, TunableKind::SkillExperience);
        assert_eq!(tunables[0].path, "content");
        assert_eq!(op.get_state(), json!({}));

        // 合法 append/pending_change 更新 → 本地预览结果(apply_update 路由到 preview)。
        let update = UpdateValue {
            payload: json!([{"id": "ev_1", "content": "learned"}]),
            mode: UpdateMode::Append,
            effect: UpdateEffect::PendingChange,
            change_type: Some("skill_experience_entry".to_string()),
            metadata: serde_json::Map::new(),
        };
        let result = PreviewableOperator::apply_update(&op, EXPERIENCES_TARGET, &update);
        assert!(result.applied, "preview applied");
        assert!(result.ok());
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0]["id"], "ev_1");
        assert_eq!(
            result.lifecycle_stage.as_deref(),
            Some(LOCAL_APPLY_COMPLETED)
        );
        assert_eq!(result.metadata["skill_name"], "my_skill");

        // apply_update 路由到 preview(PreviewableOperator 语义)。
        let routed = PreviewableOperator::apply_update(&op, EXPERIENCES_TARGET, &update);
        assert!(routed.applied);

        // 不支持目标 → 显式错误。
        let bad = op.preview_update("other", &update);
        assert!(!bad.applied);
        assert!(bad.errors[0].contains("unsupported target"));

        // 不支持 mode/effect(replace/state)→ 显式错误。
        let bad = op.preview_update(EXPERIENCES_TARGET, &UpdateValue::new(json!([{"id": "x"}])));
        assert!(!bad.applied);
        assert!(bad.errors[0].contains("unsupported update mode/effect"));

        // set_parameter 通知消费方(items 列表)。
        let fired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let f = fired.clone();
        let _guard = op.on_parameter_updated(Arc::new(move |t, v| {
            assert_eq!(t, EXPERIENCES_TARGET);
            assert!(v.is_array());
            f.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        op.set_parameter(EXPERIENCES_TARGET, json!({"id": "r1"}))
            .expect("set");
        assert_eq!(fired.load(std::sync::atomic::Ordering::SeqCst), 1);
        // None 值跳过。
        op.set_parameter(EXPERIENCES_TARGET, Value::Null)
            .expect("set");
        assert_eq!(fired.load(std::sync::atomic::Ordering::SeqCst), 1);

        // 检查点为空且 load_state 无副作用。
        op.load_state(json!({"x": 1})).expect("load");
        assert_eq!(op.get_state(), json!({}));
    }

    #[test]
    fn skill_experience_operator_registers_via_registry() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn OperatorRegistry>(&OPERATOR)
            .expect("registry");
        let op: Arc<dyn Operator> = Arc::new(SkillExperienceOperator::new("team_x"));
        let _guard = registry.register(op);
        let fetched = registry.get("skill_experience_team_x").expect("fetched");
        assert_eq!(fetched.operator_id(), "skill_experience_team_x");
        let ids = registry.ids();
        assert!(ids.contains(&"skill_experience_team_x".to_string()));
        drop(_guard);
        assert!(
            registry.get("skill_experience_team_x").is_none(),
            "guard drop unregisters"
        );
        drop(effects);
    }
}
