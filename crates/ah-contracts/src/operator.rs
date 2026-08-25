//! operator seam:自进化参数句柄(对齐 Python core/operator)。
//!
//! Operator 不是可执行单元——它是参数句柄:进化框架(optimizer/tune)经
//! set_parameter 更新参数(检查 freeze 标记),消费方(agent/workflow)经
//! on_parameter_updated 回调即时同步。get_state/load_state 支持检查点。

use serde_json::Value;

use crate::evolving::{ApplyResult, UpdateEffect, UpdateMode, UpdateValue};
use crate::seam::Seam;

/// Tunable 类型(TunableKind)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunableKind {
    Prompt,
    Continuous,
    Discrete,
    ToolSelector,
    MemorySelector,
    Text,
    /// 技能经验记录(对齐 Python kind="skill_experience")。
    SkillExperience,
}

/// 单个可调参数描述。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TunableSpec {
    pub name: String,
    pub kind: TunableKind,
    /// 参数在 operator 中的路径。
    pub path: String,
    /// 可选约束。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint: Option<Value>,
}

/// operator 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorError(pub String);

impl core::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OperatorError {}

/// 参数更新回调:消费方同步最新值。
pub type ParameterUpdated = std::sync::Arc<dyn Fn(&str, &Value) + Send + Sync>;

/// Operator Seam(Service Definition):自进化参数句柄。
///
/// 实现方(插件)管理具体参数;进化框架调用 set_parameter 更新;
/// 消费方注册 on_parameter_updated 同步。
pub trait Operator: Seam {
    /// 唯一标识(格式 {agent_id}/{kind}_{name};轨迹归因与检查点)。
    fn operator_id(&self) -> String;

    /// 可调参数(冻结参数不返回)。
    fn get_tunables(&self) -> Vec<TunableSpec>;

    /// 当前参数状态(检查点/回滚用)。
    fn get_state(&self) -> Value;

    /// 设置参数(进化更新;冻结参数跳过;成功后触发回调)。
    fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError>;

    /// 从检查点恢复(不检查 freeze)。
    fn load_state(&self, state: Value) -> Result<(), OperatorError>;

    /// 注册参数更新回调(返回可逆 guard)。
    fn on_parameter_updated(&self, callback: ParameterUpdated) -> crate::effect::Effect;

    /// 应用结构化进化更新(对齐 Python `Operator.apply_update`)。
    ///
    /// 默认兼容行为:仅支持 `replace/state` 更新,委托给 `set_parameter`
    /// 并比较前后状态判定是否实际变更;其余 mode/effect 组合返回显式错误。
    /// 可预览算子(`PreviewableOperator`)覆盖此方法路由到 `preview_update`。
    fn apply_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
        if update.mode != UpdateMode::Replace || update.effect != UpdateEffect::State {
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
                    "unsupported update mode/effect for compatibility operator: {}/{}",
                    update.mode.as_str(),
                    update.effect.as_str()
                )],
                metadata: update.metadata.clone(),
            };
        }

        let before_state = self.get_state();
        let set_result = self.set_parameter(target, update.payload.clone());
        let after_state = self.get_state();
        let mut errors = Vec::new();
        if let Err(error) = set_result {
            errors.push(error.0);
        }
        let applied = errors.is_empty() && before_state != after_state;
        ApplyResult {
            operator_id: self.operator_id(),
            target: target.to_string(),
            applied,
            mode: update.mode,
            effect: update.effect,
            value: Some(update.payload.clone()),
            records: vec![],
            change_type: update.change_type.clone(),
            lifecycle_stage: None,
            pending_change_id: None,
            errors,
            metadata: update.metadata.clone(),
        }
    }
}

/// 可预览算子(对齐 Python `PreviewableOperator`)。
///
/// 预览更新只产生本地应用结果;审批与持久化归调用方的生命周期管理器,
/// 不归算子。`apply_update` 被覆盖为直接路由到 `preview_update`。
pub trait PreviewableOperator: Operator {
    /// 应用本地预览更新,不进入 pending/持久化。
    fn preview_update(&self, target: &str, update: &UpdateValue) -> ApplyResult;

    /// 标准更新执行路由到预览语义。
    fn apply_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
        self.preview_update(target, update)
    }
}

/// Operator 注册表 Seam:按 id 注册/取回算子。
pub trait OperatorRegistry: Seam {
    /// 注册算子,返回可逆 guard。
    fn register(&self, operator: std::sync::Arc<dyn Operator>) -> crate::effect::Effect;

    /// 按 id 取回算子。
    fn get(&self, operator_id: &str) -> Option<std::sync::Arc<dyn Operator>>;

    /// 全部算子 id。
    fn ids(&self) -> Vec<String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 最小 Operator 实现(记录 state)。
    struct DummyOperator {
        state: std::sync::Mutex<Value>,
    }

    impl Seam for DummyOperator {}

    impl Operator for DummyOperator {
        fn operator_id(&self) -> String {
            "dummy/op".to_string()
        }
        fn get_tunables(&self) -> Vec<TunableSpec> {
            vec![]
        }
        fn get_state(&self) -> Value {
            self.state.lock().unwrap().clone()
        }
        fn set_parameter(&self, target: &str, value: Value) -> Result<(), OperatorError> {
            if target == "frozen" {
                return Err(OperatorError("frozen target".to_string()));
            }
            *self.state.lock().unwrap() = value;
            Ok(())
        }
        fn load_state(&self, state: Value) -> Result<(), OperatorError> {
            *self.state.lock().unwrap() = state;
            Ok(())
        }
        fn on_parameter_updated(&self, _cb: ParameterUpdated) -> crate::effect::Effect {
            crate::effect::Effect::new(|| {})
        }
    }

    fn dummy() -> DummyOperator {
        DummyOperator {
            state: std::sync::Mutex::new(json!({"v": 1})),
        }
    }

    #[test]
    fn apply_update_replace_state_applies_and_compares() {
        let op = dummy();
        let update = UpdateValue {
            payload: json!({"v": 2}),
            mode: UpdateMode::Replace,
            effect: UpdateEffect::State,
            change_type: None,
            metadata: serde_json::Map::new(),
        };
        let result = op.apply_update("x", &update);
        assert!(result.applied, "state changed -> applied");
        assert!(result.errors.is_empty());
        assert!(result.ok());

        // 相同值 → 未变更(无错误,但 applied=false → ok()=false,对齐 Python)。
        let result = op.apply_update("x", &update);
        assert!(!result.applied, "same value -> not applied");
        assert!(result.errors.is_empty(), "no error recorded");
    }

    #[test]
    fn apply_update_unsupported_mode_effect_errors() {
        let op = dummy();
        let update = UpdateValue {
            payload: json!([1]),
            mode: UpdateMode::Append,
            effect: UpdateEffect::PendingChange,
            change_type: Some("skill_experience_entry".to_string()),
            metadata: serde_json::Map::new(),
        };
        let result = op.apply_update("x", &update);
        assert!(!result.applied);
        assert!(result.errors[0].contains("unsupported update mode/effect"));
        assert_eq!(result.mode, UpdateMode::Append);
        assert_eq!(result.effect, UpdateEffect::PendingChange);
    }

    #[test]
    fn apply_update_set_parameter_error_recorded() {
        let op = dummy();
        let update = UpdateValue {
            payload: json!(1),
            mode: UpdateMode::Replace,
            effect: UpdateEffect::State,
            change_type: None,
            metadata: serde_json::Map::new(),
        };
        let result = op.apply_update("frozen", &update);
        assert!(!result.applied);
        assert_eq!(result.errors, vec!["frozen target".to_string()]);
    }

    /// PreviewableOperator 覆盖 apply_update → 路由到 preview_update。
    struct PreviewDummy {
        state: std::sync::Mutex<Value>,
    }

    impl Seam for PreviewDummy {}

    impl Operator for PreviewDummy {
        fn operator_id(&self) -> String {
            "preview/op".to_string()
        }
        fn get_tunables(&self) -> Vec<TunableSpec> {
            vec![]
        }
        fn get_state(&self) -> Value {
            self.state.lock().unwrap().clone()
        }
        fn set_parameter(&self, _t: &str, _v: Value) -> Result<(), OperatorError> {
            Ok(())
        }
        fn load_state(&self, _s: Value) -> Result<(), OperatorError> {
            Ok(())
        }
        fn on_parameter_updated(&self, _cb: ParameterUpdated) -> crate::effect::Effect {
            crate::effect::Effect::new(|| {})
        }
    }

    impl PreviewableOperator for PreviewDummy {
        fn preview_update(&self, target: &str, update: &UpdateValue) -> ApplyResult {
            ApplyResult {
                operator_id: self.operator_id(),
                target: target.to_string(),
                applied: true,
                mode: update.mode,
                effect: update.effect,
                value: Some(update.payload.clone()),
                records: vec![update.payload.clone()],
                change_type: update.change_type.clone(),
                lifecycle_stage: Some("local_apply_completed".to_string()),
                pending_change_id: None,
                errors: vec![],
                metadata: update.metadata.clone(),
            }
        }
    }

    #[test]
    fn previewable_operator_routes_apply_to_preview() {
        let op = PreviewDummy {
            state: std::sync::Mutex::new(json!({})),
        };
        let update = UpdateValue::normalize(json!([{"id": "ev_1"}]), Some("experiences"));
        let result = PreviewableOperator::apply_update(&op, "experiences", &update);
        assert!(result.applied);
        assert_eq!(result.records.len(), 1);
        assert_eq!(
            result.lifecycle_stage.as_deref(),
            Some("local_apply_completed")
        );
        // 确认走的是 preview 路径:normalize 出 append/pending_change 也照常应用。
        assert_eq!(result.mode, UpdateMode::Append);
        assert_eq!(result.effect, UpdateEffect::PendingChange);
    }

    #[test]
    fn tunable_kind_skill_experience_serializes() {
        let kind = TunableKind::SkillExperience;
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            json!("skill_experience")
        );
    }
}
