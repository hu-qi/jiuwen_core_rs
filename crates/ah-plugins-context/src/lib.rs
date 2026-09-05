//! # ah-plugins-context
//!
//! 真实上下文引擎(对应 openjiuwen/core 的 context_engine):
//! - token 预算内组装:从会话日志 derive 消息,预算充足时原样返回;
//! - 压缩:超出预算时把最早消息压缩为摘要并 reinject 为首条 system 消息;
//!   摘要默认为确定性摘录(mock 桩不参与摘要,文档策略);真实 LLM provider
//!   注册时尝试 LLM 总结,失败显式回退摘录;
//! - offload:压缩出的早期消息持久化为 JSONL(完整历史始终在会话日志)。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ah_contracts::context::{
    AssembledContext, ContextEngine, ContextError, ContextSummary, SummarySource,
};
use ah_contracts::keys::{CONTEXT, LLM, PROMPT_ATTACHMENT_STORE, TOKENIZER};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_attachment::{PromptAttachmentStore, render};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::SessionLog;
use ah_contracts::tokenizer::Tokenizer;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::json;

/// 每个 session 的压缩摘要缓存;完整消息仍由 SessionLog 持有,这里仅缓存可复用摘要。
#[derive(Default)]
struct SessionMemoryManager {
    summaries: Mutex<HashMap<(String, u64, usize), ContextSummary>>,
}

impl SessionMemoryManager {
    fn get(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        compressed_tokens: usize,
    ) -> Option<ContextSummary> {
        let key = (
            session_id.to_string(),
            message_fingerprint(messages),
            compressed_tokens,
        );
        self.summaries.lock().unwrap().get(&key).cloned()
    }

    fn put(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        compressed_tokens: usize,
        summary: ContextSummary,
    ) {
        let key = (
            session_id.to_string(),
            message_fingerprint(messages),
            compressed_tokens,
        );
        self.summaries.lock().unwrap().insert(key, summary);
    }
}

fn message_fingerprint(messages: &[ChatMessage]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for message in messages {
        (message.role as u8).hash(&mut hasher);
        message.content.hash(&mut hasher);
        message.tool_call_id.hash(&mut hasher);
        for image in &message.images {
            image.mime_type.hash(&mut hasher);
            image.data.hash(&mut hasher);
        }
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                call.id.hash(&mut hasher);
                call.name.hash(&mut hasher);
                call.arguments.to_string().hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

pub struct ContextEngineImpl {
    /// 可选 LLM(真实 provider 用于总结;mock 桩被排除,文档策略)。
    llm: Option<Arc<dyn ModelProvider>>,
    /// 可选精确 tokenizer(注册时用于 estimate_tokens,否则启发式)。
    tokenizer: Option<Arc<dyn Tokenizer>>,
    /// 可选 prompt attachment 窗口注入器;附件只进入本次模型窗口,不写入 session。
    prompt_attachments: Option<Arc<dyn PromptAttachmentStore>>,
    /// 会话记忆摘要缓存。
    session_memory: SessionMemoryManager,
    /// offload 文件目录。
    offload_dir: PathBuf,
}

impl ContextEngineImpl {
    /// 构造引擎;llm 为 None 时仅确定性摘录。
    pub fn new(llm: Option<Arc<dyn ModelProvider>>, offload_dir: impl Into<PathBuf>) -> Self {
        Self {
            llm,
            tokenizer: None,
            prompt_attachments: None,
            session_memory: SessionMemoryManager::default(),
            offload_dir: offload_dir.into(),
        }
    }

    /// 挂载精确 tokenizer(上下文预算更精确)。
    pub fn with_tokenizer(mut self, tokenizer: Arc<dyn Tokenizer>) -> Self {
        self.tokenizer = Some(tokenizer);
        self
    }

    /// 挂载 prompt attachment 窗口注入器。
    pub fn with_prompt_attachments(mut self, store: Arc<dyn PromptAttachmentStore>) -> Self {
        self.prompt_attachments = Some(store);
        self
    }

    fn message_tokens(&self, message: &ChatMessage) -> usize {
        let mut tokens = self.estimate_tokens(&message.content);
        // 图像 token 由 provider/分辨率决定;固定保守预算避免图像消息逃逸窗口上限。
        tokens += message.images.len() * 256;
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

    /// 压缩最早 messages 为摘要,并通过 session memory manager 复用相同前缀结果。
    async fn compress(
        &self,
        session_id: &str,
        older: &[ChatMessage],
        compressed_tokens: usize,
    ) -> ContextSummary {
        if let Some(summary) = self
            .session_memory
            .get(session_id, older, compressed_tokens)
        {
            return summary;
        }
        let excerpt = excerpt_text(older);
        let summary = if let Some(llm) = &self.llm
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
            match llm.chat(request).await {
                Ok(resp) if !resp.content.trim().is_empty() => ContextSummary {
                    summary: truncate(&resp.content, 400),
                    source: SummarySource::Llm,
                    compressed_messages: older.len(),
                    compressed_tokens,
                },
                _ => ContextSummary {
                    summary: truncate(&excerpt, 400),
                    source: SummarySource::Excerpt,
                    compressed_messages: older.len(),
                    compressed_tokens,
                },
            }
        } else {
            ContextSummary {
                summary: truncate(&excerpt, 400),
                source: SummarySource::Excerpt,
                compressed_messages: older.len(),
                compressed_tokens,
            }
        };
        self.session_memory
            .put(session_id, older, compressed_tokens, summary.clone());
        summary
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

        let last_user = messages
            .iter()
            .rposition(|message| message.role == ChatRole::User)
            .unwrap_or(messages.len() - 1);
        let rounds = completed_rounds(&messages);
        let mut keep_start = last_user;
        if let Some((latest_start, latest_end)) = rounds.last().copied() {
            // 保留最近一个完整 dialogue round;若当前轮未结束,同时保留当前 user
            // 及其后续消息,避免切断 assistant tool-call/tool-result 对。
            keep_start = if latest_end >= last_user {
                latest_start
            } else {
                latest_start.min(last_user)
            };
        }

        let mut kept_tokens = self.messages_tokens(&messages[keep_start..]);
        // 预算允许时仅按完整 round 向前扩展,绝不从 tool-call block 中间切开。
        for (round_start, _) in rounds.iter().rev() {
            if *round_start >= keep_start {
                continue;
            }
            let candidate_tokens = self.messages_tokens(&messages[*round_start..]);
            if candidate_tokens > budget_tokens {
                break;
            }
            keep_start = *round_start;
            kept_tokens = candidate_tokens;
        }
        let kept = messages[keep_start..].to_vec();
        let older = messages[..keep_start].to_vec();
        let older_tokens = total.saturating_sub(kept_tokens);
        let summary = self.compress(session.id(), &older, older_tokens).await;
        Ok((kept, older, summary))
    }
}
fn completed_rounds(messages: &[ChatMessage]) -> Vec<(usize, usize)> {
    let mut rounds = Vec::new();
    let mut start = None;
    for (index, message) in messages.iter().enumerate() {
        if message.role == ChatRole::User && start.is_none() {
            start = Some(index);
        }
        if message.role == ChatRole::Assistant
            && message.tool_calls.as_ref().is_none_or(Vec::is_empty)
            && let Some(round_start) = start.take()
        {
            rounds.push((round_start, index));
        }
    }
    rounds
}

impl Seam for ContextEngineImpl {}

#[async_trait]
impl ContextEngine for ContextEngineImpl {
    fn estimate_tokens(&self, text: &str) -> usize {
        // 注册了精确 tokenizer 时使用它;否则启发式(字符/4 + 词数)。
        if let Some(tokenizer) = &self.tokenizer
            && let Ok(count) = tokenizer.count(text)
        {
            return count.max(1);
        }
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
        let summary = if summary.compressed_messages > 0 {
            Some(summary)
        } else {
            None
        };
        if let Some(store) = &self.prompt_attachments {
            let attachments = store.collect_for_session(session.id());
            let rendered = render(
                &attachments,
                ah_contracts::prompt_attachment::DEFAULT_MAX_PROMPT_ATTACHMENT_CHARS,
                ah_contracts::prompt_attachment::DEFAULT_MAX_RENDERED_CHARS,
            );
            if !rendered.is_empty() {
                messages.push(ChatMessage::new(ChatRole::User, rendered));
            }
        }
        let total_tokens = self.messages_tokens(&messages);
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
        let mut engine = ContextEngineImpl::new(llm, self.offload_dir.clone());
        // 精确 tokenizer 可选:注册时用于更精确的预算估计。
        if let Some(tokenizer) = ctx.service::<dyn Tokenizer>(&TOKENIZER) {
            engine = engine.with_tokenizer(tokenizer);
        }
        // prompt attachment 为最终窗口 mutator:只进入当前请求,不写 session 日志。
        if let Some(store) = ctx.service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE) {
            engine = engine.with_prompt_attachments(store);
        }
        Ok(vec![ctx.register(
            CONTEXT,
            Arc::new(engine) as Arc<dyn ContextEngine>,
        )])
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

    #[test]
    fn estimate_uses_tokenizer_when_registered() {
        let root = std::env::temp_dir().join(format!("ah-ctx-tok-{}", std::process::id()));
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_tokenizer::TokenizerPlugin),
            StdArc::new(ContextPlugin::new(root.join("offload"))),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let tokenizer = ctx
            .service::<dyn ah_contracts::tokenizer::Tokenizer>(&TOKENIZER)
            .expect("tokenizer");

        let text = "你好世界 working";
        // 精确 tokenizer 参与估计(与启发式不同,CJK 每字 1 token + 子词合并)。
        let expected = tokenizer.count(text).expect("count").max(1);
        assert_eq!(
            engine.estimate_tokens(text),
            expected,
            "tokenizer-backed estimate"
        );

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

    #[test]
    fn image_attachments_consume_context_budget() {
        let engine = ContextEngineImpl::new(None, std::env::temp_dir());
        let plain = ChatMessage::new(ChatRole::User, "screen");
        let image = ChatMessage::user_with_image(
            "screen",
            ah_contracts::llm::ChatImage::new("image/png", "AAAA"),
        );
        assert_eq!(
            engine.message_tokens(&image),
            engine.message_tokens(&plain) + 256
        );
    }
    #[tokio::test]
    async fn assemble_never_splits_a_completed_tool_round() {
        let root = std::env::temp_dir().join(format!("ah-ctx-round-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("round").expect("create");
        for (label, suffix) in [("old", "history"), ("recent", "keep")] {
            log.append(
                SessionEventKind::User,
                json!({"content": format!("{label} request {}", "word ".repeat(20))}),
            )
            .expect("user");
            log.append(
                SessionEventKind::Assistant,
                json!({"tool_calls": [{"id": format!("{label}-call"), "name": "list_dir", "arguments": {"path": "."}}]}),
            )
            .expect("assistant tool call");
            log.append(
                SessionEventKind::ToolResult,
                json!({"tool_call_id": format!("{label}-call"), "output": format!("{suffix} tool output {}", "word ".repeat(20))}),
            )
            .expect("tool result");
            log.append(
                SessionEventKind::Assistant,
                json!({"content": format!("{label} final {}", "word ".repeat(20))}),
            )
            .expect("assistant final");
        }
        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let assembled = engine.assemble(log.as_ref(), 230).await.expect("assemble");
        assert!(assembled.summary.is_some(), "old dialogue should compress");
        let visible = assembled
            .messages
            .iter()
            .filter(|message| message.role != ChatRole::System)
            .collect::<Vec<_>>();
        assert_eq!(visible.len(), 4, "the recent round remains whole");
        assert_eq!(
            visible
                .iter()
                .map(|message| message.role)
                .collect::<Vec<_>>(),
            vec![
                ChatRole::User,
                ChatRole::Assistant,
                ChatRole::Tool,
                ChatRole::Assistant
            ],
        );
        assert!(visible[0].content.contains("recent"));
        assert!(visible[2].content.contains("keep"));
        assert!(visible.iter().all(|message| {
            message.tool_call_id.as_deref() != Some("old-call")
                && message
                    .tool_calls
                    .as_ref()
                    .is_none_or(|calls| calls.iter().all(|call| call.id != "old-call"))
        }));
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn assemble_appends_prompt_attachments_after_context_window() {
        let root = std::env::temp_dir().join(format!("ah-ctx-attachment-{}", std::process::id()));
        let ctx = Context::new();
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let effects = ctx
            .mount_all(vec![
                StdArc::new(ah_plugins_mock::MockPlugin),
                StdArc::new(ah_plugins_tools::ToolsPlugin),
                StdArc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
                StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    &default_path,
                    &session_dir,
                )),
                StdArc::new(ah_plugins_prompt_attachment::PromptAttachmentPlugin),
                StdArc::new(ContextPlugin::new(root.join("offload"))),
            ])
            .expect("mount");
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let log = manager.create("attachment").expect("create");
        log.append(SessionEventKind::User, json!({"content": "request"}))
            .expect("user");
        let store = ctx
            .service::<dyn ah_contracts::prompt_attachment::PromptAttachmentStore>(
                &ah_contracts::keys::PROMPT_ATTACHMENT_STORE,
            )
            .expect("attachment store");
        store
            .add_section(
                "attachment",
                "runtime",
                "only for this model call",
                ah_contracts::prompt_attachment::PromptAttachmentKind::Runtime,
                "test",
                1,
                None,
                "text/plain",
                None,
            )
            .expect("attachment");
        let engine = ctx.service::<dyn ContextEngine>(&CONTEXT).expect("context");
        let assembled = engine.assemble(log.as_ref(), 1000).await.expect("assemble");
        assert_eq!(
            assembled.messages.last().expect("attachment").role,
            ChatRole::User
        );
        assert!(
            assembled
                .messages
                .last()
                .expect("attachment")
                .content
                .contains("<system-reminder>")
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
}
