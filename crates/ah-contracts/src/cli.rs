//! cli seam:终端渲染(Claude Code 风格,对齐 Python harness/cli/ui/renderer.py)。
//!
//! 渲染块(CliChunk)对齐 Python 的 OutputSchema chunk 类型:
//! llm_output / llm_reasoning / answer / message / tool_call / tool_result /
//! todo.updated / controller_output。渲染器把事件投影为块,再渲染为终端行。

use serde_json::Value;

use crate::seam::Seam;
use crate::session::SessionEvent;

/// 待办状态(todo checkbox 渲染)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

/// 待办项。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

/// 渲染块:一次终端输出单元。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum CliChunk {
    /// LLM 生成文本(累积进结果)。
    LlmOutput { text: String },
    /// 推理/思考(默认隐藏)。
    LlmReasoning { text: String },
    /// 最终回答(避免重复:仅在没有 llm_output 时展示)。
    Answer { text: String },
    /// 系统/状态消息。
    Message { text: String },
    /// 工具执行开始。
    ToolCall { name: String, arguments: Value },
    /// 工具执行结果。
    ToolResult {
        name: String,
        arguments: Value,
        result: String,
    },
    /// todo 列表变化(checkbox 渲染)。
    TodoUpdated { items: Vec<TodoItem> },
    /// 控制器任务失败信息。
    ControllerOutput { text: String },
}

/// cli 渲染错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError(pub String);

impl core::fmt::Display for CliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

/// cli 渲染 Seam(Service Definition):事件 → 终端行。
///
/// 实现方(插件)提供具体渲染格式;消费方(交互 CLI)只依赖本 trait。
pub trait CliRenderer: Seam {
    /// 渲染一个块为终端行(无 ANSI,可测试;CLI 可自行加色)。
    fn render(&self, chunk: &CliChunk) -> Vec<String>;

    /// 从会话事件投影为渲染块(纯函数,无副作用)。
    fn chunks_from_event(&self, event: &SessionEvent) -> Vec<CliChunk>;

    /// 渲染待办列表为 checkbox 行 + 进度摘要。
    fn render_todo(&self, items: &[TodoItem]) -> (Vec<String>, String);
}
