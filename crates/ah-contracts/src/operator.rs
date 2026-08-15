//! operator seam:自进化参数句柄(对齐 Python core/operator)。
//!
//! Operator 不是可执行单元——它是参数句柄:进化框架(optimizer/tune)经
//! set_parameter 更新参数(检查 freeze 标记),消费方(agent/workflow)经
//! on_parameter_updated 回调即时同步。get_state/load_state 支持检查点。

use serde_json::Value;

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
