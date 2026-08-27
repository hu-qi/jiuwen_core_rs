//! # ah-plugins-cli
//!
//! 真实 cli 渲染器:把会话事件/工具执行投影为 Claude Code 风格终端行
//! (对齐 Python harness/cli/ui/renderer.py + tool_display.py + todo_render.py):
//! - ● ToolName(args)            —— 工具调用开始
//! - ⎿  summary                  —— 工具结果摘要(Read N lines / Wrote to path ...)
//! - ☑/◐/☐/☒ + 进度摘要          —— todo checkbox 渲染
//! - ⚙ message / 推理(默认隐藏)   —— 系统消息与推理
//! - ✗ controller 失败信息        —— 控制器错误渲染

use std::sync::Arc;

use ah_contracts::cli::{CliChunk, CliRenderer, TodoItem, TodoStatus};
use ah_contracts::keys::CLI_RENDERER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::{SessionEvent, SessionEventKind};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 内部工具名 → 友好显示名(对齐 Python _TOOL_DISPLAY_NAMES)。
const DISPLAY_NAMES: &[(&str, &str)] = &[
    ("read_file", "Read"),
    ("write_file", "Write"),
    ("edit_file", "Edit"),
    ("run_shell", "Bash"),
    ("bash", "Bash"),
    ("list_dir", "LS"),
    ("ls", "LS"),
    ("web_search", "WebSearch"),
    ("web_fetch", "WebFetch"),
    ("run_code", "Code"),
    ("search_knowledge", "Search"),
    ("ingest_knowledge", "Ingest"),
    ("remember", "Remember"),
    ("recall", "Recall"),
    ("delegate_task", "Delegate"),
    ("mcp_call_tool", "MCP"),
    ("todo_create", "TodoWrite"),
    ("todo_modify", "TodoWrite"),
    ("todo_list", "TodoList"),
];

/// 真实终端渲染器。
pub struct TerminalRenderer;

impl Seam for TerminalRenderer {}

impl TerminalRenderer {
    fn display_name(name: &str) -> String {
        DISPLAY_NAMES
            .iter()
            .find(|(internal, _)| *internal == name)
            .map(|(_, display)| (*display).to_string())
            .unwrap_or_else(|| {
                name.replace('_', " ")
                    .split_whitespace()
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            Some(first) => {
                                let rest: String = chars.collect();
                                format!("{}{}", first.to_ascii_uppercase(), rest)
                            }
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
    }

    fn arg_str(name: &str, arguments: &Value) -> String {
        let args = match arguments {
            Value::Object(map) => map.clone(),
            _ => return String::new(),
        };
        let get = |key: &str| -> String {
            args.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match name {
            "read_file" => {
                let path = get("path");
                if path.is_empty() {
                    String::new()
                } else {
                    let limit = args.get("limit").and_then(Value::as_u64);
                    match limit {
                        Some(n) => format!("{path}, limit={n}"),
                        None => path,
                    }
                }
            }
            "write_file" | "edit_file" => get("path"),
            "run_shell" | "bash" => {
                let command = get("command");
                if command.chars().count() > 60 {
                    format!("{}...", command.chars().take(57).collect::<String>())
                } else {
                    command
                }
            }
            "list_dir" | "ls" => {
                let path = get("path");
                if path.is_empty() {
                    ".".to_string()
                } else {
                    path
                }
            }
            "web_search" | "search_knowledge" => get("query"),
            "web_fetch" => get("url"),
            _ => {
                // fallback:第一个参数值(字符串解包)。
                let first = args
                    .values()
                    .next()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                if first.chars().count() > 60 {
                    format!("{}...", first.chars().take(57).collect::<String>())
                } else {
                    first
                }
            }
        }
    }

    fn result_summary(name: &str, arguments: &Value, result: &str) -> String {
        if result.is_empty() {
            return "Done".to_string();
        }
        let count_lines = |s: &str| s.lines().count();
        match name {
            "read_file" => format!("Read {} lines", count_lines(result)),
            "write_file" => {
                let path = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let lines = count_lines(result);
                if lines > 0 {
                    format!("Wrote {lines} lines to {path}")
                } else {
                    format!("Wrote to {path}")
                }
            }
            "edit_file" => {
                let first = result.lines().next().unwrap_or("Edited file");
                if first.chars().count() <= 80 {
                    first.to_string()
                } else {
                    "Edited file".to_string()
                }
            }
            "run_shell" | "bash" => {
                let lines: Vec<&str> = result.lines().collect();
                if lines.len() == 1 && lines[0].chars().count() <= 80 {
                    lines[0].to_string()
                } else {
                    let first = lines
                        .first()
                        .map(|l| l.chars().take(60).collect::<String>())
                        .unwrap_or_default();
                    format!("{first}... (+{} lines)", lines.len().saturating_sub(1))
                }
            }
            "list_dir" | "ls" => {
                let count = result.lines().filter(|l| !l.trim().is_empty()).count();
                format!("Found {count} entries")
            }
            "search_knowledge" | "recall" => {
                let count = result.lines().filter(|l| !l.trim().is_empty()).count();
                format!("Found {count} hits")
            }
            _ => {
                let first = result.lines().next().unwrap_or("Done");
                if first.chars().count() <= 80 {
                    first.to_string()
                } else {
                    format!("{}...", first.chars().take(77).collect::<String>())
                }
            }
        }
    }

    fn todo_icon(status: TodoStatus) -> char {
        match status {
            TodoStatus::Completed => '☑',
            TodoStatus::InProgress => '◐',
            TodoStatus::Pending => '☐',
            TodoStatus::Cancelled => '☒',
        }
    }
}

impl CliRenderer for TerminalRenderer {
    fn render(&self, chunk: &CliChunk) -> Vec<String> {
        match chunk {
            CliChunk::LlmOutput { text } => {
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![format!("● {text}")]
                }
            }
            // 推理默认隐藏(与 Python show_reasoning=False 对齐)。
            CliChunk::LlmReasoning { .. } => Vec::new(),
            CliChunk::Answer { text } => {
                if text.is_empty() {
                    Vec::new()
                } else {
                    vec![text.to_string()]
                }
            }
            CliChunk::Message { text } => vec![format!("⚙ {text}")],
            CliChunk::ToolCall { name, arguments } => {
                let display = Self::display_name(name);
                let args = Self::arg_str(name, arguments);
                if args.is_empty() {
                    vec![format!("● {display}")]
                } else {
                    vec![format!("● {display}({args})")]
                }
            }
            CliChunk::ToolResult {
                name,
                arguments,
                result,
            } => {
                let summary = Self::result_summary(name, arguments, result);
                vec![format!("  ⎿  {summary}")]
            }
            CliChunk::TodoUpdated { items } => {
                let (lines, _) = self.render_todo(items);
                lines
            }
            CliChunk::ControllerOutput { text } => vec![format!("✗ {text}")],
        }
    }

    fn chunks_from_event(&self, event: &SessionEvent) -> Vec<CliChunk> {
        match event.kind {
            SessionEventKind::User => Vec::new(),
            SessionEventKind::System => {
                let text = event
                    .payload
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                vec![CliChunk::Message {
                    text: text.to_string(),
                }]
            }
            SessionEventKind::Assistant => {
                if let Some(calls) = event.payload.get("tool_calls").and_then(Value::as_array) {
                    calls
                        .iter()
                        .filter_map(|call| {
                            Some(CliChunk::ToolCall {
                                name: call.get("name")?.as_str()?.to_string(),
                                arguments: call.get("arguments")?.clone(),
                            })
                        })
                        .collect()
                } else {
                    let text = event
                        .payload
                        .get("content")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    vec![CliChunk::LlmOutput {
                        text: text.to_string(),
                    }]
                }
            }
            SessionEventKind::ToolResult => {
                let output = event
                    .payload
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                vec![CliChunk::ToolResult {
                    name: String::new(),
                    arguments: Value::Null,
                    result: output.to_string(),
                }]
            }
            SessionEventKind::AgentStep
            | SessionEventKind::AgentInterrupted
            | SessionEventKind::AgentCanceled
            | SessionEventKind::AgentTimedOut => Vec::new(),
        }
    }

    fn render_todo(&self, items: &[TodoItem]) -> (Vec<String>, String) {
        let mut counts = [0usize; 4];
        let lines: Vec<String> = items
            .iter()
            .map(|item| {
                let icon = Self::todo_icon(item.status);
                let idx = match item.status {
                    TodoStatus::Completed => 0,
                    TodoStatus::InProgress => 1,
                    TodoStatus::Pending => 2,
                    TodoStatus::Cancelled => 3,
                };
                counts[idx] += 1;
                format!("  ⎿  {icon} {}", item.content)
            })
            .collect();
        let mut parts: Vec<String> = Vec::new();
        if counts[0] > 0 {
            parts.push(format!("✓{}", counts[0]));
        }
        if counts[1] > 0 {
            parts.push(format!("◐{}", counts[1]));
        }
        if counts[2] > 0 {
            parts.push(format!("☐{}", counts[2]));
        }
        let summary = if parts.is_empty() {
            "No tasks".to_string()
        } else {
            parts.join(" ")
        };
        (lines, summary)
    }
}

/// cli 渲染插件:提供 cli-renderer seam。
pub struct CliPlugin;

impl Plugin for CliPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-cli"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CLI_RENDERER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let renderer: Arc<dyn CliRenderer> = Arc::new(TerminalRenderer);
        Ok(vec![ctx.register(CLI_RENDERER, renderer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::session::SessionEventKind;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(CliPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn event(kind: SessionEventKind, payload: Value, seq: u64) -> SessionEvent {
        SessionEvent {
            seq,
            timestamp_ms: 0,
            kind,
            payload,
        }
    }

    #[test]
    fn renderer_is_registered_and_renders_tool_call() {
        let root = std::env::temp_dir().join(format!("ah-cli-reg-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let renderer = ctx
            .service::<dyn CliRenderer>(&CLI_RENDERER)
            .expect("cli renderer");
        let lines = renderer.render(&CliChunk::ToolCall {
            name: "read_file".to_string(),
            arguments: json!({ "path": "src/main.rs" }),
        });
        assert_eq!(lines, vec!["● Read(src/main.rs)"]);

        // 工具结果摘要。
        let summary = renderer.render(&CliChunk::ToolResult {
            name: "read_file".to_string(),
            arguments: json!({}),
            result: "line1
line2
line3"
                .to_string(),
        });
        assert_eq!(summary, vec!["  ⎿  Read 3 lines"]);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn todo_rendering_uses_checkboxes_and_summary() {
        let renderer = TerminalRenderer;
        let items = vec![
            TodoItem {
                content: "write tests".to_string(),
                status: TodoStatus::Completed,
            },
            TodoItem {
                content: "run clippy".to_string(),
                status: TodoStatus::InProgress,
            },
            TodoItem {
                content: "commit".to_string(),
                status: TodoStatus::Pending,
            },
        ];
        let (lines, summary) = renderer.render_todo(&items);
        assert_eq!(lines[0], "  ⎿  ☑ write tests");
        assert_eq!(lines[1], "  ⎿  ◐ run clippy");
        assert_eq!(lines[2], "  ⎿  ☐ commit");
        assert_eq!(summary, "✓1 ◐1 ☐1");

        let (empty, summary) = renderer.render_todo(&[]);
        assert!(empty.is_empty());
        assert_eq!(summary, "No tasks");
    }

    #[test]
    fn event_projection_maps_assistant_and_tool_results() {
        let renderer = TerminalRenderer;
        // Assistant 带 tool_calls → ToolCall 块。
        let calls = event(
            SessionEventKind::Assistant,
            json!({
                "tool_calls": [
                    { "id": "1", "name": "read_file", "arguments": { "path": "a.txt" } },
                    { "id": "2", "name": "list_dir", "arguments": { "path": "." } },
                ]
            }),
            0,
        );
        let chunks = renderer.chunks_from_event(&calls);
        assert_eq!(chunks.len(), 2);
        assert!(matches!(&chunks[0], CliChunk::ToolCall { name, .. } if name == "read_file"));

        // Assistant 纯文本 → LlmOutput。
        let text = event(
            SessionEventKind::Assistant,
            json!({ "content": "hello" }),
            1,
        );
        let chunks = renderer.chunks_from_event(&text);
        assert!(matches!(&chunks[0], CliChunk::LlmOutput { text } if text == "hello"));

        // ToolResult → ToolResult 块。
        let result = event(
            SessionEventKind::ToolResult,
            json!({ "tool_call_id": "1", "output": "x\ny" }),
            2,
        );
        let chunks = renderer.chunks_from_event(&result);
        assert!(matches!(&chunks[0], CliChunk::ToolResult { result, .. } if result == "x\ny"));

        // 端到端:会话事件流 → 渲染行。
        let lines: Vec<String> = [calls, text, result]
            .iter()
            .flat_map(|e| renderer.chunks_from_event(e))
            .flat_map(|c| renderer.render(&c))
            .collect();
        assert!(lines.iter().any(|l| l.starts_with("● Read(a.txt)")));
        assert!(lines.iter().any(|l| l.starts_with("● LS(.)")));
        assert!(lines.iter().any(|l| l.starts_with("● hello")));
    }

    #[test]
    fn arg_and_result_formatting_edge_cases() {
        let renderer = TerminalRenderer;
        // 长命令截断。
        let long_cmd = "x".repeat(80);
        let lines = renderer.render(&CliChunk::ToolCall {
            name: "run_shell".to_string(),
            arguments: json!({ "command": long_cmd }),
        });
        assert!(lines[0].ends_with("...)"));

        // 空结果 → Done。
        let lines = renderer.render(&CliChunk::ToolResult {
            name: "write_file".to_string(),
            arguments: json!({ "path": "out.txt" }),
            result: String::new(),
        });
        assert_eq!(lines, vec!["  ⎿  Done"]);

        // list_dir 默认路径。
        let lines = renderer.render(&CliChunk::ToolCall {
            name: "list_dir".to_string(),
            arguments: json!({}),
        });
        assert_eq!(lines, vec!["● LS(.)"]);

        // 未知工具名 → 下划线转标题。
        let lines = renderer.render(&CliChunk::ToolCall {
            name: "my_tool".to_string(),
            arguments: json!({ "x": "1" }),
        });
        assert_eq!(lines, vec!["● My Tool(1)"]);
    }
}
