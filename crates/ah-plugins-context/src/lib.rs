//! # ah-plugins-context
//!
//! 真实上下文引擎(对应 openjiuwen/core 的 context_engine):
//! - token 预算内组装:从会话日志 derive 消息,预算充足时原样返回;
//! - 压缩:超出预算时把最早消息压缩为摘要并 reinject 为首条 system 消息;
//!   摘要默认为确定性摘录(mock 桩不参与摘要,文档策略);真实 LLM provider
//!   注册时尝试 LLM 总结,失败显式回退摘录;
//! - offload:压缩出的早期消息持久化为 JSONL(完整历史始终在会话日志)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::context::{
    AssembledContext, ContextEngine, ContextError, ContextSummary, SummarySource,
};
use ah_contracts::keys::{CONTEXT, LLM};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::SessionLog;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::json;

/// 真实上下文引擎。
pub struct ContextEngineImpl {
    /// 可选 LLM(真实 provider 用于总结;mock 桩被排除,文档策略)。
    llm: Option<Arc<dyn ModelProvider>>,
    /// offload 文件目录。
    offload_dir: PathBuf,
}

impl ContextEngineImpl {
    /// 构造引擎;llm 为 None 时仅确定性摘录。
    pub fn new(llm: Option<Arc<dyn ModelProvider>>, offload_dir: impl Into<PathBuf>) -> Self {
        Self {
            llm,
            offload_dir: offload_dir.into(),
        }
    }

    fn message_tokens(&self, message: &ChatMessage) -> usize {
        let mut tokens = self.estimate_tokens(&message.content);
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                tokens += self.estimate_tokens(&call.name);
                tokens += self.estimate_tokens(&call.arguments.to_string());
            }
        }
        tokens
    }

    fn messages_tokens(&self, messages: &[ChatMessage]) -> usize {
        messages.iter().map(|m| self.message_tokens(m)).sum()
    }

    /// 压缩最早 messages 为摘要:确定性摘录为基底;真实 LLM 可用时尝试总结。
    async fn compress(&self, older: &[ChatMessage], compressed_tokens: usize) -> ContextSummary {
        let excerpt = excerpt_text(older);
        if let Some(llm) = &self.llm
            && llm.name() != "mock"
        {
            let joined = older
                .iter()
                .map(|m| format!("{}: {}", role_str(m.role), m.content))
                .collect::<Vec<_>>()
                .join("\n");
            let request = ModelRequest {
                messages: vec![
                    ChatMessage::new(
                        ChatRole::System,
                        "Summarize the conversation so far in a few sentences, keeping key facts, decisions and open questions.",
                    ),
                    ChatMessage::new(ChatRole::User, joined),
                ],
                ..Default::default()
            };
            if let Ok(resp) = llm.chat(request).await
                && !resp.content.trim().is_empty()
            {
                return ContextSummary {
                    summary: truncate(&resp.content, 400),
                    source: SummarySource::Llm,
                    compressed_messages: older.len(),
                    compressed_tokens,
                };
            }
        }
        ContextSummary {
            summary: truncate(&excerpt, 400),
            source: SummarySource::Excerpt,
            compressed_messages: older.len(),
            compressed_tokens,
        }
    }

    /// 压缩并返回 (保留消息, 被压缩消息, 摘要)。
    async fn compress_session(
        &self,
        session: &dyn SessionLog,
        budget_tokens: usize,
    ) -> Result<(Vec<ChatMessage>, Vec<ChatMessage>, ContextSummary), ContextError> {
        let messages = session.derive_messages();
        if messages.is_empty() {
            return Ok((
                Vec::new(),
                Vec::new(),
                ContextSummary {
                    summary: String::new(),
                    source: SummarySource::Excerpt,
                    compressed_messages: 0,
                    compressed_tokens: 0,
                },
            ));
        }
        let total = self.messages_tokens(&messages);
        if total <= budget_tokens {
            return Ok((
                messages,
                Vec::new(),
                ContextSummary {
                    summary: String::new(),
                    source: SummarySource::Excerpt,
                    compressed_messages: 0,
                    compressed_tokens: 0,
                },
            ));
        }
        // 保留最近一次用户请求及之后的消息(模型必须看到当前请求);
        // 再向前扩展,直到预算耗尽。
        let last_user = messages
            .iter()
            .rposition(|m| m.role == ChatRole::User)
            .unwrap_or(messages.len() - 1);
        let mut start = last_user;
        let mut kept_tokens = 0usize;
        for idx in (0..=last_user).rev() {
            let t = self.message_tokens(&messages[idx]);
            if kept_tokens + t > budget_tokens {
                break;
            }
            kept_tokens += t;
            start = idx;
        }
        let kept = messages[start..].to_vec();
        let older = messages[..start].to_vec();
        let older_tokens = total - kept_tokens;
        let summary = self.compress(&older, older_tokens).await;
        Ok((kept, older, summary))
    }
}

impl Seam for ContextEngineImpl {}

#[async_trait]
impl ContextEngine for ContextEngineImpl {
    fn estimate_tokens(&self, text: &str) -> usize {
        let chars = text.chars().count();
        let words = text.split_whitespace().count();
        (chars / 4 + words).max(1)
    }

    async fn assemble(
        &self,
        session: &dyn SessionLog,
        budget_tokens: usize,
    ) -> Result<AssembledContext, ContextError> {
        let (kept, _older, summary) = self.compress_session(session, budget_tokens).await?;
        let mut messages = kept;
        if summary.compressed_messages > 0 {
            messages.insert(
                0,
                ChatMessage::new(
                    ChatRole::System,
                    format!("Previous context (compressed): {}", summary.summary),
                ),
            );
        }
        let total_tokens = self.messages_tokens(&messages);
        let summary = if summary.compressed_messages > 0 {
            Some(summary)
        } else {
            None
        };
        Ok(AssembledContext {
            messages,
            summary,
            total_tokens,
            budget_tokens,
        })
    }

    async fn offload(
        &self,
        session: &dyn SessionLog,
        budget_tokens: usize,
    ) -> Result<(ContextSummary, String), ContextError> {
        let (_kept, older, summary) = self.compress_session(session, budget_tokens).await?;
        if older.is_empty() {
            return Err(ContextError(
                "nothing to offload (context within budget)".to_string(),
            ));
        }
        std::fs::create_dir_all(&self.offload_dir)
            .map_err(|e| ContextError(format!("create offload dir failed: {e}")))?;
        let path = self.offload_dir.join("offload.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| ContextError(format!("open offload file failed: {e}")))?;
        use std::io::Write;
        for message in &older {
            let line = json!({
                "role": role_str(message.role),
                "content": message.content,
            });
            writeln!(file, "{line}")
                .map_err(|e| ContextError(format!("write offload failed: {e}")))?;
        }
        Ok((summary, path.to_string_lossy().into_owned()))
    }
}

fn role_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

/// 每条消息取前 80 字符的角色标注摘录。
fn excerpt_text(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| format!("{}: {}", role_str(m.role), truncate(&m.content, 80)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

/// context 插件:可选注入 LLM,提供 context seam。
pub struct ContextPlugin {
    offload_dir: PathBuf,
}

impl ContextPlugin {
    /// 以 offload 目录创建插件。
    pub fn new(offload_dir: impl Into<PathBuf>) -> Self {
        Self {
            offload_dir: offload_dir.into(),
        }
    }
}

impl Plugin for ContextPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-context"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CONTEXT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        // LLM 可选:未注册或为 mock 桩时仅用确定性摘录(文档策略)。
        let llm = ctx.service::<dyn ModelProvider>(&LLM);
        let engine: Arc<dyn ContextEngine> =
            Arc::new(ContextEngineImpl::new(llm, self.offload_dir.clone()));
        Ok(vec![ctx.register(CONTEXT, engine)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{CONTEXT, SESSION_MANAGER};
    use ah_contracts::session::{SessionEventKind, SessionManager};
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
            StdArc::new(ContextPlugin::new(root.join("offload"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    fn fill_session(manager: &dyn SessionManager, id: &str, turns: usize, words: usize) {
        let log = manager.create(id).expect("create");
        for i in 0..turns {
            log.append(
                SessionEventKind::User,
                json!({ "content": format!("turn {i} request with {}", "word ".repeat(words)) }),
            )
            .expect("user");
            log.append(
                SessionEventKind::Assistant,
                json!({ "content": format!("turn {i} answer with {}", "word ".repeat(words)) }),
            )
            .expect("assistant");
        }
    }

    #[tokio::test]
    async fn estimate_tokens_is_deterministic() {
        let root = std::env::temp_dir().join(format!("ah-ctx-est-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let tokens = engine.estimate_tokens("hello world");
        assert!(tokens >= 1, "minimum one token");
        assert_eq!(
            tokens,
            "hello world".chars().count() / 4 + 2,
            "chars/4 + words"
        );
        assert_eq!(engine.estimate_tokens(""), 1, "empty is at least one");

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn assemble_within_budget_returns_all_messages() {
        let root = std::env::temp_dir().join(format!("ah-ctx-fit-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        fill_session(manager.as_ref(), "fit", 2, 5);

        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let log = manager.open("fit").expect("open");
        let assembled = engine
            .assemble(log.as_ref(), 100_000)
            .await
            .expect("assemble");
        assert!(assembled.summary.is_none(), "no compression within budget");
        assert_eq!(assembled.messages.len(), 4, "2 turns = 4 messages");
        assert_eq!(assembled.messages[0].role, ChatRole::User);

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn assemble_over_budget_compresses_and_reinjects() {
        let root = std::env::temp_dir().join(format!("ah-ctx-cmp-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        // 5 轮 × 每轮两条长消息。
        fill_session(manager.as_ref(), "cmp", 5, 60);

        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let log = manager.open("cmp").expect("open");
        let assembled = engine.assemble(log.as_ref(), 80).await.expect("assemble");
        let summary = assembled.summary.expect("compressed");
        assert!(summary.compressed_messages > 0, "early messages compressed");
        assert_eq!(
            summary.source,
            SummarySource::Excerpt,
            "mock stub excluded from LLM summary"
        );
        // 摘要被 reinject 为首条 system 消息。
        assert_eq!(assembled.messages[0].role, ChatRole::System);
        assert!(
            assembled.messages[0]
                .content
                .contains("Previous context (compressed)")
        );
        // 至少保留最新一条(最后一条 user)。
        assert!(
            assembled
                .messages
                .iter()
                .any(|m| m.content.contains("turn 4 request"))
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn offload_persists_compressed_messages_to_jsonl() {
        let root = std::env::temp_dir().join(format!("ah-ctx-off-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        fill_session(manager.as_ref(), "off", 4, 40);

        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let log = manager.open("off").expect("open");
        let (summary, path) = engine.offload(log.as_ref(), 60).await.expect("offload");
        assert!(summary.compressed_messages > 0);
        assert!(std::path::Path::new(&path).exists(), "offload file written");
        let lines = std::fs::read_to_string(&path).expect("read offload");
        let count = lines.lines().count();
        assert_eq!(
            count, summary.compressed_messages,
            "one line per compressed message"
        );
        // 每行都是合法 JSON。
        for line in lines.lines() {
            serde_json::from_str::<serde_json::Value>(line).expect("valid json line");
        }

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
