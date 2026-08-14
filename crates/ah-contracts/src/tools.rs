//! tools seam:模型可见工具与工具注册表。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::effect::Effect;
use crate::seam::Seam;

/// 工具错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError(pub String);

impl core::fmt::Display for ToolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ToolError {}

/// 模型可见工具(Service Definition)。
///
/// 实现方(插件)提供具体工具;消费方(agent 循环、workflow 组件)
/// 通过 [`ToolRegistry`] 按名调用,不 import 具体实现。
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// 工具稳定名称(模型可见,如 `echo`、`add`)。
    fn name(&self) -> &'static str;

    /// 工具描述(进入 prompt 组装)。
    fn description(&self) -> &'static str;

    /// 参数 JSON schema(进入模型请求的 tools 字段);默认宽松 object。
    fn parameters(&self) -> Value {
        json!({ "type": "object" })
    }

    /// 执行工具,返回 JSON 结果。
    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError>;
}

/// 工具注册表 seam:工具集合的 Service Definition。
///
/// 消费方通过 `tools` 服务键解析注册表,再按名取工具/调用。
#[async_trait]
pub trait ToolRegistry: Seam {
    /// 注册一个工具,返回可逆注册 guard(drop 即移除)。
    fn register(&self, tool: Arc<dyn Tool>) -> Effect;

    /// 按名取工具。
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;

    /// 当前全部工具名(无序)。
    fn names(&self) -> Vec<String>;

    /// 按名调用工具。
    async fn invoke(&self, name: &str, arguments: Value) -> Result<Value, ToolError>;
}

// ------------------------------------------------------------------
// 工具执行管线事件(event-catalog: tools/pre-execute, tools/post-execute)
// ------------------------------------------------------------------

/// `tools/pre-execute` 事件:工具执行前发布(waterfall)。
///
/// 监听器(rails/鉴权/预算)可:
/// - 调用 Next::next 委托下游(可改写参数);
/// - 直接返回拒绝决策(短路,工具不执行)。
#[derive(Clone, Debug)]
pub struct ToolInvocation {
    pub name: String,
    pub arguments: Value,
}

impl crate::event::Event for ToolInvocation {
    const ID: &'static str = "tools/pre-execute";
}

/// `tools/pre-execute` waterfall 的决策值。
#[derive(Clone, Debug)]
pub struct ToolDecision {
    /// 是否允许执行。
    pub allow: bool,
    /// 拒绝原因(allow=false 时必填)。
    pub reason: Option<String>,
    /// 实际执行的参数(监听器可改写)。
    pub arguments: Value,
}

impl ToolDecision {
    /// 允许执行。
    pub fn allow(arguments: Value) -> Self {
        Self {
            allow: true,
            reason: None,
            arguments,
        }
    }

    /// 拒绝执行。
    pub fn deny(arguments: Value, reason: impl Into<String>) -> Self {
        Self {
            allow: false,
            reason: Some(reason.into()),
            arguments,
        }
    }
}

/// `tools/post-execute` 事件:工具执行完成后发布(serial)。
#[derive(Clone, Debug)]
pub struct ToolExecuted {
    pub name: String,
    pub arguments: Value,
    pub output: Value,
    pub elapsed_ms: u64,
}

impl crate::event::Event for ToolExecuted {
    const ID: &'static str = "tools/post-execute";
}
