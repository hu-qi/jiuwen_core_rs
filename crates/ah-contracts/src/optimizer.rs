//! optimizer seam:真实文本梯度优化(对齐 Python agent_evolving/optimizer)。
//!
//! 与 Operator 参数句柄配合:backward 从评估问题推导文本梯度(真实规则),
//! step 经 OperatorRegistry.set_parameter 应用到算子(冻结参数拒绝并记录)。

use crate::evolving::Evaluation;
use crate::seam::Seam;

/// 一条文本梯度:算子参数的目标更新。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TextualGradient {
    pub operator_id: String,
    pub parameter: String,
    /// 梯度文本(如修正指令)。
    pub gradient: String,
    /// 关联的问题(来源 evaluation.issues)。
    pub issue: String,
}

/// 一次更新的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UpdateResult {
    pub operator_id: String,
    pub parameter: String,
    /// 是否应用成功。
    pub applied: bool,
    /// 未应用原因(冻结/无此参数/算子缺失)。
    pub reason: Option<String>,
}

/// optimizer 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizerError(pub String);

impl core::fmt::Display for OptimizerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OptimizerError {}

/// optimizer Seam(Service Definition):文本梯度优化。
///
/// 实现方(插件)提供真实 backward/step;消费方(trainer/auto-harness)
/// 只依赖本 trait。
pub trait Optimizer: Seam {
    /// backward:从评估结果推导文本梯度(仅失败/待改进信号,真实规则)。
    /// 返回按 operator/parameter 的目标更新。
    fn backward(&self, evaluations: &[Evaluation]) -> Vec<TextualGradient>;

    /// step:把梯度经 OperatorRegistry 应用到算子。
    /// 返回每条更新的应用结果(冻结/缺失显式记录,不静默)。
    fn step(
        &self,
        gradients: &[TextualGradient],
        registry: &dyn crate::operator::OperatorRegistry,
    ) -> Vec<UpdateResult>;

    /// 便捷:backward + step 一次完成(供 trainer 单步调用)。
    fn apply_updates(
        &self,
        evaluations: &[Evaluation],
        registry: &dyn crate::operator::OperatorRegistry,
    ) -> Result<Vec<UpdateResult>, OptimizerError>;
}
