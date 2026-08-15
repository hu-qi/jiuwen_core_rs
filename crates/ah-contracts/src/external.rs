//! external seam:外部 CLI agent 接入(对齐 Python agent_teams/external)。
//!
//! 一个外部 agent 进程(第三方 CLI,如 claude/codex/openclaw)以一等团队成员身份
//! 接入团队:运行时按每个 CLI 的启动知识(CliAgentAdapter)拉起子进程,把任务
//! 文本送入 stdin、从 stdout 读叙述直到"轮完成"信号,支持中途 steer 与 abort。
//!
//! 两种运行时风味(同一 seam,由 adapter.supports_stdin_injection 选择):
//! - StreamingCliRuntime:一条长驻子进程;每轮把入站文本写 stdin,读到完成标记。
//! - ReinvokeCliRuntime:每轮新子进程(prompt 作 argv 传入),读到 EOF。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 输入 framing 策略(任务文本如何写入 CLI stdin)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    /// 纯文本一行。
    Text,
    /// Codex proto:JSONL submission 一行。
    CodexProto,
}

/// 轮完成判定策略(如何从 stdout 判断一轮结束)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStrategy {
    /// 读到 EOF。
    None,
    /// 行包含指定前缀即完成。
    MarkerPrefix { prefix: String },
}

/// 每 CLI 启动知识(纯数据,对齐 Python CliAgentAdapter)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CliAgentAdapter {
    /// adapter 键(如 "generic" / "codex")。
    pub name: String,
    /// 完整启动 argv(binary + flags)。
    pub command: Vec<String>,
    /// 输入 framing。
    pub input_format: InputFormat,
    /// 轮完成判定。
    pub completion: CompletionStrategy,
    /// true = 长驻 stdin 流式(StreamingCliRuntime);
    /// false = 每轮新进程(ReinvokeCliRuntime)。
    pub supports_stdin_injection: bool,
    /// 单发:prompt 经 <flag> <prompt> 传入;None = 尾部位置参数。
    pub prompt_flag: Option<String>,
    /// 单发:起手轮加 <flag> <session_id>。
    pub session_flag: Option<String>,
    /// 单发:后续轮用 <flag> <session_id> 续会话。
    pub resume_flag: Option<String>,
    /// 单发:首轮之后追加的连续性参数。
    pub continue_args: Vec<String>,
    /// 无输出(静默)上限秒;超时判为挂死。
    pub inactivity_timeout_s: u64,
}

impl CliAgentAdapter {
    /// 通用文本流式 adapter:任务写 stdin,读到 marker 前缀行即完成。
    pub fn generic_streaming(marker: &str) -> Self {
        Self {
            name: "generic".to_string(),
            command: vec![],
            input_format: InputFormat::Text,
            completion: CompletionStrategy::MarkerPrefix {
                prefix: marker.to_string(),
            },
            supports_stdin_injection: true,
            prompt_flag: None,
            session_flag: None,
            resume_flag: None,
            continue_args: vec![],
            inactivity_timeout_s: 30,
        }
    }
}

/// 一轮的输出。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExternalCliTurn {
    /// stdout 叙述行(含完成标记,消费方自行剥离)。
    pub narration: Vec<String>,
    /// 是否因无输出超时终止。
    pub timed_out: bool,
}

/// 外部 CLI 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCliError(pub String);

impl core::fmt::Display for ExternalCliError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExternalCliError {}

/// 外部 CLI 运行时 Seam(Service Definition):进程级成员交互表面。
///
/// 实现方(插件)按 adapter 拉真实子进程;消费方(团队调度)只依赖本 trait。
#[async_trait]
pub trait ExternalCliRuntime: Seam {
    /// 启动成员(流式:拉起长驻子进程;单发:预校验命令,惰性拉起)。
    fn start(&self, session_id: &str) -> Result<(), ExternalCliError>;

    /// 送一轮任务并读输出直到轮完成(或超时)。
    async fn send(&self, task: &str) -> Result<ExternalCliTurn, ExternalCliError>;

    /// 中途 steer(仅流式;单发缓冲到下一轮)。
    async fn steer(&self, message: &str) -> Result<(), ExternalCliError>;

    /// 中止当前轮(杀子进程)。
    fn abort(&self) -> Result<(), ExternalCliError>;

    /// 停止成员并回收进程。
    fn stop(&self) -> Result<(), ExternalCliError>;

    /// 是否仍在运行。
    fn is_running(&self) -> bool;
}

/// 外部 agent 加入团队的连接描述(对齐 TeamJoinDescriptor;纯数据)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamJoinDescriptor {
    pub session_id: String,
    pub team_name: String,
    pub member_name: String,
    pub role: String,
    /// "member" = 一等成员(真实 teammate 工具);"operator" = 控制面。
    pub scope: String,
    pub language: String,
    pub dispatch_mode: String,
    /// 环境变量名(对齐 OPENJIUWEN_TEAM_JOIN)。
    pub env_var: String,
}

impl TeamJoinDescriptor {
    /// 默认环境变量名。
    pub const ENV: &'static str = "OPENJIUWEN_TEAM_JOIN";

    /// 编码为单行 JSON(注入环境变量)。
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// 从 JSON 解码。
    pub fn decode(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// 从当前环境读取(env_var 字段指定变量名)。
    pub fn from_env(&self) -> Option<Self> {
        let value = std::env::var(&self.env_var).ok()?;
        Self::decode(&value).ok()
    }
}

/// 解析 codex_proto 输入:把任务文本包成 JSONL submission。
pub fn codex_proto_line(task: &str) -> String {
    // 对齐 codex proto:一行 JSON 含 "type":"submission" 与 "content"。
    let value = serde_json::json!({
        "type": "submission",
        "content": task,
    });
    value.to_string()
}

/// 从 codex proto 输出行提取叙述文本(纯函数)。
pub fn codex_narration(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str) == Some("message") {
        value
            .get("payload")
            .and_then(|p| p.get("content"))
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    }
}
