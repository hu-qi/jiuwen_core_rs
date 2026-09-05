//! # ah-plugins-external-client
//!
//! Real process-boundary external member client (aligned with
//! openjiuwen/agent_teams/external/client.py): `ExternalTeamClient` lets an
//! external agent act as a first-class team member via shared DB + messager.
//! This crate provides:
//! - `ExternalClientFactory` implementing the `external-client` seam: builds
//!   a client bound to a `TeamJoinDescriptor` (MCP server / external CLI each
//!   connection constructs one).
//! - `ExternalTeamClientImpl`: descriptor projections, session-context
//!   binding, idempotent connect/close, `fetch_inbox` / `read_inbox` /
//!   `watch` over an injected `ExternalInboxSource`.
//!
//! Rendering composes the `external-format` seam (render_message /
//! render_task_board, per-message reply hints) + `team-message` seam
//! (framework template expansion) + `team-i18n` (board headers), mirroring
//! client.py's `read_inbox` assembly. Data access (unread messages /
//! mark-read / task board / row lookups) is the `ExternalInboxSource` seam
//! injected at `connect` — the team runtime owns the real DB.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::external_client::{
    ExternalClientError, ExternalInboxSource, ExternalTeamClient, ExternalTeamClientFactory,
    InboxMessage, InboxObserver, InboxView, compose_inbox_text,
};
use ah_contracts::external_format::ExternalFormat;
use ah_contracts::inbound_render::InboundRender;
use ah_contracts::keys::{
    EXTERNAL_CLIENT, EXTERNAL_FORMAT, INBOUND_RENDER, TEAM_CONTEXT, TEAM_I18N, TEAM_MESSAGE,
    TEAM_PROMPT_LOADER, TIMEFMT,
};
use ah_contracts::messager::Messager;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_context::TeamSessionContext;
use ah_contracts::team_i18n::{KEY_HITT_SILENCE_NOTE, Language, TeamI18n};
use ah_contracts::team_join_descriptor::{Scope, TeamJoinDescriptor};
use ah_contracts::team_message::TeamMessage;
use ah_contracts::team_prompts::TeamPromptLoader;
use ah_contracts::team_schema::TeamTopic;
use ah_contracts::timefmt::Timefmt;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;

/// 当前毫秒 UTC epoch(对齐 `get_current_time`)。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

struct WatchSubscription {
    messager: Arc<dyn Messager>,
    topics: Vec<String>,
}

impl Drop for WatchSubscription {
    fn drop(&mut self) {
        for topic in &self.topics {
            self.messager.unsubscribe(topic);
        }
    }
}

/// 真实客户端实现:描述符投影 + 状态机 + 收件箱组装。
pub struct ExternalTeamClientImpl {
    descriptor: TeamJoinDescriptor,
    /// 渲染 seam(经工厂注入)。
    format: Arc<dyn ExternalFormat>,
    message: Arc<dyn TeamMessage>,
    loader: Arc<dyn TeamPromptLoader>,
    i18n: Arc<dyn TeamI18n>,
    session: Arc<dyn TeamSessionContext>,
    timefmt: Arc<dyn Timefmt>,
    render: Arc<dyn InboundRender>,
    /// 消息传输 seam;fetch/read 不依赖,watch 需要它。
    messager: Option<Arc<dyn Messager>>,
    /// 数据源(connect 时注入)。
    source: Mutex<Option<Arc<dyn ExternalInboxSource>>>,
    connected: AtomicBool,
}

impl ExternalTeamClientImpl {
    /// 绑定描述符与渲染 seam 构建客户端(未连接)。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        descriptor: TeamJoinDescriptor,
        format: Arc<dyn ExternalFormat>,
        message: Arc<dyn TeamMessage>,
        loader: Arc<dyn TeamPromptLoader>,
        i18n: Arc<dyn TeamI18n>,
        session: Arc<dyn TeamSessionContext>,
        timefmt: Arc<dyn Timefmt>,
        render: Arc<dyn InboundRender>,
        messager: Option<Arc<dyn Messager>>,
    ) -> Self {
        Self {
            descriptor,
            format,
            message,
            loader,
            i18n,
            session,
            timefmt,
            render,
            messager,
            source: Mutex::new(None),
            connected: AtomicBool::new(false),
        }
    }

    fn require_source(&self) -> Result<Arc<dyn ExternalInboxSource>, ExternalClientError> {
        self.source
            .lock()
            .expect("external client source mutex poisoned")
            .clone()
            .ok_or_else(|| {
                ExternalClientError(
                    "ExternalTeamClient is not connected; call connect() first".to_string(),
                )
            })
    }

    fn scope_str(&self) -> &'static str {
        match self.descriptor.scope {
            Scope::Member => "member",
            Scope::Operator => "operator",
        }
    }

    /// 语言代码(cn/en;空 → 默认 cn,对齐 `descriptor.language or "cn"`)。
    fn language(&self) -> &str {
        if self.descriptor.language.is_empty() {
            "cn"
        } else {
            &self.descriptor.language
        }
    }

    /// 展开框架模板消息正文(对齐 `_expand_template_bodies`)。
    ///
    /// 普通消息(无 meta / 非模板)不产生 body 条目;模板消息经
    /// team-message `expand` 渲染;模板缺失或展开失败 → fallback 行
    /// (对齐 Python `expand_message` 的异常降级)。
    async fn expand_template_bodies(
        &self,
        messages: &[InboxMessage],
        source: &dyn ExternalInboxSource,
    ) -> HashMap<String, String> {
        let mut bodies = HashMap::new();
        for msg in messages {
            let Some(meta) = self.message.parse_meta(msg.meta.as_deref()) else {
                continue;
            };
            // 模板正文加载失败 → fallback 行。
            let template_content = match self.loader.load(&meta.template, self.language()) {
                Ok(content) => content,
                Err(_) => {
                    bodies.insert(
                        msg.view.message_id.clone(),
                        self.message.fallback_line(&meta),
                    );
                    continue;
                }
            };
            // 引用行读取(refs 中的 task/member;缺失行 → expand 内部降级 fallback)。
            let task = match meta.refs.get("task") {
                Some(task_id) => source.get_task(task_id).await,
                None => None,
            };
            let member = match meta.refs.get("member") {
                Some(name) => source.get_member(name).await,
                None => None,
            };
            let expanded = self.message.expand(
                &msg.view.content,
                msg.meta.as_deref(),
                &template_content,
                task.as_ref(),
                member.as_ref(),
            );
            if expanded.is_template {
                bodies.insert(msg.view.message_id.clone(), expanded.body);
            }
        }
        bodies
    }
}

impl Seam for ExternalTeamClientImpl {}

#[async_trait]
impl ExternalTeamClient for ExternalTeamClientImpl {
    fn session_id(&self) -> &str {
        &self.descriptor.session_id
    }

    fn language(&self) -> &str {
        &self.descriptor.language
    }

    fn member_name(&self) -> &str {
        &self.descriptor.member_name
    }

    fn team_name(&self) -> &str {
        &self.descriptor.team_name
    }

    fn is_leader(&self) -> bool {
        self.descriptor.role == "leader"
    }

    fn is_human_agent(&self) -> bool {
        self.descriptor.role == "human_agent"
    }

    fn scope(&self) -> &str {
        self.scope_str()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn connect(&self, source: Arc<dyn ExternalInboxSource>) -> Result<(), ExternalClientError> {
        if self.is_connected() {
            return Ok(());
        }
        // 绑定会话上下文(对齐 connect 里的 set_session_id + set_language)。
        self.session
            .set_session_id(&self.descriptor.session_id)
            .map_err(|e| ExternalClientError(e.0))?;
        self.i18n.set_language(if self.descriptor.language == "en" {
            Language::En
        } else {
            Language::Cn
        });
        *self
            .source
            .lock()
            .expect("external client source mutex poisoned") = Some(source);
        self.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn close(&self) -> Result<(), ExternalClientError> {
        *self
            .source
            .lock()
            .expect("external client source mutex poisoned") = None;
        self.connected.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn bind_session_context(&self) -> Result<(), ExternalClientError> {
        self.session
            .set_session_id(&self.descriptor.session_id)
            .map_err(|e| ExternalClientError(e.0))?;
        Ok(())
    }

    async fn fetch_inbox(&self, mark_read: bool) -> Result<InboxView, ExternalClientError> {
        let source = self.require_source()?;
        let mut messages = source.unread_direct_messages(self.member_name()).await;
        messages.extend(source.unread_broadcast_messages(self.member_name()).await);
        if mark_read {
            for msg in &messages {
                source
                    .mark_message_read(&msg.view.message_id, self.member_name())
                    .await;
            }
        }
        let tasks = source.list_tasks().await;
        Ok(InboxView { messages, tasks })
    }

    async fn read_inbox(&self, mark_read: bool) -> Result<String, ExternalClientError> {
        let view = self.fetch_inbox(mark_read).await?;
        let now = now_ms();
        let mut parts: Vec<String> = Vec::new();
        if !view.messages.is_empty() {
            let source = self.require_source()?;
            let bodies = self
                .expand_template_bodies(&view.messages, source.as_ref())
                .await;
            // 逐消息渲染,按发送者取 reply hint(对齐 format.py render_message 的
            // `reply_hint_for(message.from_member_name)` 每消息提示)。
            let silence_note = if self.is_human_agent() {
                self.i18n
                    .t(KEY_HITT_SILENCE_NOTE, &[])
                    .map(|s| s.to_string())
                    .ok()
            } else {
                None
            };
            let rendered: Vec<String> = view
                .messages
                .iter()
                .map(|m| {
                    let reply_hint = if self.is_human_agent() {
                        None
                    } else {
                        Some(self.i18n.reply_hint_for(&m.view.from_member_name))
                    };
                    self.format.render_message(
                        &m.view,
                        self.is_human_agent(),
                        now,
                        bodies.get(&m.view.message_id).map(|s| s.as_str()),
                        reply_hint.as_deref(),
                        silence_note.as_deref(),
                        self.render.as_ref(),
                        self.timefmt.as_ref(),
                    )
                })
                .collect();
            if !rendered.is_empty() {
                parts.push(rendered.join("\n\n"));
            }
        }
        let leader_header = self
            .i18n
            .t("dispatcher.leader_task_board", &[])
            .unwrap_or_else(|_| "任务看板".to_string());
        let teammate_header = self
            .i18n
            .t("dispatcher.teammate_task_list", &[])
            .unwrap_or_else(|_| "当前任务".to_string());
        let unassigned_marker = self
            .i18n
            .t("dispatcher.task_unassigned_marker", &[])
            .unwrap_or_else(|_| " (待领取)".to_string());
        let board = self.format.render_task_board(
            &view.tasks,
            self.is_leader(),
            now,
            &leader_header,
            &teammate_header,
            &unassigned_marker,
            self.render.as_ref(),
            self.timefmt.as_ref(),
        );
        let messages_text = parts.first().map(String::as_str).unwrap_or("");
        Ok(compose_inbox_text(messages_text, &board))
    }

    async fn watch(&self, observer: &dyn InboxObserver) -> Result<(), ExternalClientError> {
        self.require_source()?;
        let messager = self.messager.clone().ok_or_else(|| {
            ExternalClientError(
                "watch requires messager topic subscription (MESSAGE/TASK)".to_string(),
            )
        })?;
        let topics = vec![
            TeamTopic::Message.build(&self.descriptor.session_id, self.team_name()),
            TeamTopic::Task.build(&self.descriptor.session_id, self.team_name()),
            format!("team:{}:messages", self.team_name()),
            format!("team:{}:task", self.team_name()),
        ];
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<()>();
        for topic in &topics {
            let sender = sender.clone();
            messager.subscribe(
                topic,
                Arc::new(move |_event| {
                    let _ = sender.send(());
                }),
            );
        }
        drop(sender);
        let _subscription = WatchSubscription { messager, topics };
        while receiver.recv().await.is_some() {
            let view = self.fetch_inbox(true).await?;
            if !view.is_empty() {
                observer.on_inbox(view).await;
            }
        }
        Ok(())
    }
}

/// 客户端工厂:按描述符构建客户端(对齐 `ExternalTeamClient(descriptor)`)。
pub struct ExternalClientFactory {
    format: Arc<dyn ExternalFormat>,
    message: Arc<dyn TeamMessage>,
    loader: Arc<dyn TeamPromptLoader>,
    i18n: Arc<dyn TeamI18n>,
    session: Arc<dyn TeamSessionContext>,
    timefmt: Arc<dyn Timefmt>,
    render: Arc<dyn InboundRender>,
    messager: Option<Arc<dyn Messager>>,
}

impl ExternalClientFactory {
    /// 从上下文解析全部渲染 seam;缺失 → 显式错误(不静默)。
    pub fn from_ctx(ctx: &Context) -> Result<Self, PluginError> {
        let missing = |key: &str| PluginError::Apply {
            plugin: "ah-plugins-external-client",
            message: format!("seam {key} not registered for external client"),
        };
        Ok(Self {
            format: ctx
                .service::<dyn ExternalFormat>(&EXTERNAL_FORMAT)
                .ok_or_else(|| missing("external-format"))?,
            message: ctx
                .service::<dyn TeamMessage>(&TEAM_MESSAGE)
                .ok_or_else(|| missing("team-message"))?,
            loader: ctx
                .service::<dyn TeamPromptLoader>(&TEAM_PROMPT_LOADER)
                .ok_or_else(|| missing("team-prompt-loader"))?,
            i18n: ctx
                .service::<dyn TeamI18n>(&TEAM_I18N)
                .ok_or_else(|| missing("team-i18n"))?,
            session: ctx
                .service::<dyn TeamSessionContext>(&TEAM_CONTEXT)
                .ok_or_else(|| missing("team-context"))?,
            timefmt: ctx
                .service::<dyn Timefmt>(&TIMEFMT)
                .ok_or_else(|| missing("timefmt"))?,
            render: ctx
                .service::<dyn InboundRender>(&INBOUND_RENDER)
                .ok_or_else(|| missing("inbound-render"))?,
            messager: ctx.service::<dyn Messager>(&ah_contracts::keys::MESSAGER),
        })
    }
}

impl Seam for ExternalClientFactory {}

impl ExternalTeamClientFactory for ExternalClientFactory {
    fn build(&self, descriptor: &TeamJoinDescriptor) -> Arc<dyn ExternalTeamClient> {
        Arc::new(ExternalTeamClientImpl::new(
            descriptor.clone(),
            self.format.clone(),
            self.message.clone(),
            self.loader.clone(),
            self.i18n.clone(),
            self.session.clone(),
            self.timefmt.clone(),
            self.render.clone(),
            self.messager.clone(),
        ))
    }
}

/// external-client 插件:注册 `external-client` seam(工厂)。
pub struct ExternalClientPlugin;

impl Plugin for ExternalClientPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-external-client"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![EXTERNAL_CLIENT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let factory = ExternalClientFactory::from_ctx(ctx)?;
        let svc: Arc<dyn ExternalTeamClientFactory> = Arc::new(factory);
        Ok(vec![ctx.register(EXTERNAL_CLIENT, svc)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::external_format::{MessageView, TaskLineView};
    use ah_hub::plugin::DynPlugin;
    use ah_plugins_external_format::ExternalFormatPlugin;
    use ah_plugins_inbound_render::InboundRenderer;
    use ah_plugins_team_i18n::TeamI18nPlugin;
    use ah_plugins_team_message::TeamMessagePlugin;
    use ah_plugins_team_prompts::TeamPromptsPlugin;
    use ah_plugins_timefmt::TimefmtService;
    use std::sync::Arc as StdArc;

    /// 内存收件箱数据源(测试真实路径,非 mock)。
    struct MemInboxSource {
        direct: Vec<InboxMessage>,
        broadcast: Vec<InboxMessage>,
        tasks: Vec<TaskLineView>,
        read: Mutex<Vec<String>>,
    }

    impl MemInboxSource {
        fn new(
            direct: Vec<InboxMessage>,
            broadcast: Vec<InboxMessage>,
            tasks: Vec<TaskLineView>,
        ) -> Self {
            Self {
                direct,
                broadcast,
                tasks,
                read: Mutex::new(Vec::new()),
            }
        }
    }

    impl Seam for MemInboxSource {}

    #[async_trait]
    impl ExternalInboxSource for MemInboxSource {
        async fn unread_direct_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            self.direct.clone()
        }

        async fn unread_broadcast_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            self.broadcast.clone()
        }

        async fn mark_message_read(&self, message_id: &str, _member_name: &str) {
            self.read.lock().unwrap().push(message_id.to_string());
        }

        async fn list_tasks(&self) -> Vec<TaskLineView> {
            self.tasks.clone()
        }

        async fn get_task(&self, _task_id: &str) -> Option<ah_contracts::team_message::TaskView> {
            None
        }

        async fn get_member(
            &self,
            _member_name: &str,
        ) -> Option<ah_contracts::team_message::MemberView> {
            None
        }
    }

    fn descriptor() -> TeamJoinDescriptor {
        TeamJoinDescriptor {
            session_id: "s1".to_string(),
            team_name: "t1".to_string(),
            member_name: "dev-1".to_string(),
            role: "teammate".to_string(),
            scope: Scope::Member,
            language: "cn".to_string(),
            dispatch_mode: ah_contracts::team_join_descriptor::DispatchMode::Autonomous,
            teammate_mode: ah_contracts::team_join_descriptor::TeammateMode::BuildMode,
            db_config: ah_contracts::team_join_descriptor::JoinDbConfig::default(),
            transport_config: ah_contracts::team_join_descriptor::JoinTransportConfig::default(),
            workspace_path: None,
        }
    }

    fn msg(id: &str, from: &str, content: &str) -> InboxMessage {
        InboxMessage {
            view: MessageView {
                broadcast: false,
                timestamp: 1_700_000_000_000,
                from_member_name: from.to_string(),
                message_id: id.to_string(),
                content: content.to_string(),
            },
            meta: None,
        }
    }

    fn task(id: &str, status: &str) -> TaskLineView {
        TaskLineView {
            task_id: id.to_string(),
            title: format!("Title {id}"),
            content: "body".to_string(),
            status: status.to_string(),
            assignee: None,
            updated_at: Some(1_700_000_000_000),
        }
    }

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        // inbound-render + timefmt 是 external-client 的 apply 时依赖,须先注册。
        let render: Arc<dyn InboundRender> = StdArc::new(InboundRenderer);
        let timefmt: Arc<dyn Timefmt> = StdArc::new(TimefmtService::with_offset(0));
        let e1 = ctx.register(INBOUND_RENDER, render);
        let e2 = ctx.register(TIMEFMT, timefmt);
        let messager = ah_contracts::messager::create_messager(
            ah_contracts::messager::MessagerTransportConfig {
                node_id: Some("external-test".into()),
                ..Default::default()
            },
        )
        .expect("messager");
        let e3 = ctx.register(ah_contracts::keys::MESSAGER, messager);
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_team_context::TeamContextPlugin),
            StdArc::new(TeamPromptsPlugin),
            StdArc::new(TeamMessagePlugin),
            StdArc::new(TeamI18nPlugin),
            StdArc::new(ExternalFormatPlugin),
            StdArc::new(ExternalClientPlugin),
        ];
        let mut effects = ctx.mount_all(plugins).expect("mount");
        effects.extend([e1, e2, e3]);
        (ctx, effects)
    }

    #[tokio::test]
    async fn watch_notifies_and_unsubscribes_on_cancellation() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let source: Arc<dyn ExternalInboxSource> = StdArc::new(MemInboxSource::new(
            vec![msg("watch-1", "alice", "hello")],
            vec![],
            vec![],
        ));
        client.connect(source).expect("connect");
        let views = StdArc::new(Mutex::new(Vec::<InboxView>::new()));
        let observer = RecordingObserver {
            views: views.clone(),
        };
        let watch_client = client.clone();
        let watch_task = tokio::spawn(async move { watch_client.watch(&observer).await });
        tokio::task::yield_now().await;
        let messager = ctx
            .service::<dyn ah_contracts::messager::Messager>(&ah_contracts::keys::MESSAGER)
            .expect("messager seam");
        let topic = ah_contracts::team_schema::TeamTopic::Message.build("s1", "t1");
        messager.publish(&topic, serde_json::json!({"event": "message"}));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if !views.lock().unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("watch callback");
        assert_eq!(views.lock().unwrap().len(), 1);
        let task_topic = ah_contracts::team_schema::TeamTopic::Task.build("s1", "t1");
        messager.publish(&task_topic, serde_json::json!({"event": "task"}));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if views.lock().unwrap().len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("task watch callback");
        assert_eq!(views.lock().unwrap().len(), 2);

        watch_task.abort();
        let _ = watch_task.await;
        let count = views.lock().unwrap().len();
        messager.publish(&topic, serde_json::json!({"event": "after-cancel"}));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(views.lock().unwrap().len(), count);
        drop(effects);
    }

    struct RecordingObserver {
        views: StdArc<Mutex<Vec<InboxView>>>,
    }

    #[async_trait]
    impl InboxObserver for RecordingObserver {
        async fn on_inbox(&self, view: InboxView) {
            self.views.lock().unwrap().push(view);
        }
    }

    #[tokio::test]
    async fn watch_skips_empty_inbox() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let source: Arc<dyn ExternalInboxSource> =
            StdArc::new(MemInboxSource::new(vec![], vec![], vec![]));
        client.connect(source).expect("connect");
        let views = StdArc::new(Mutex::new(Vec::<InboxView>::new()));
        let observer = RecordingObserver {
            views: views.clone(),
        };
        let watch_client = client.clone();
        let watch_task = tokio::spawn(async move { watch_client.watch(&observer).await });
        tokio::task::yield_now().await;
        let messager = ctx
            .service::<dyn ah_contracts::messager::Messager>(&ah_contracts::keys::MESSAGER)
            .expect("messager seam");
        let topic = ah_contracts::team_schema::TeamTopic::Task.build("s1", "t1");
        messager.publish(&topic, serde_json::json!({"event": "empty-task"}));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(views.lock().unwrap().is_empty());
        watch_task.abort();
        let _ = watch_task.await;
        drop(effects);
    }

    #[tokio::test]
    async fn factory_builds_client_and_connect_is_idempotent() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        assert_eq!(client.member_name(), "dev-1");
        assert_eq!(client.team_name(), "t1");
        assert_eq!(client.scope(), "member");
        assert!(!client.is_leader());
        assert!(!client.is_human_agent());
        assert!(!client.is_connected());

        let source: Arc<dyn ExternalInboxSource> = StdArc::new(MemInboxSource::new(
            vec![msg("m1", "alice", "hello")],
            vec![],
            vec![task("t1", "pending")],
        ));
        client.connect(source).expect("connect");
        assert!(client.is_connected());
        // 幂等:重复 connect 为 no-op。
        let source2: Arc<dyn ExternalInboxSource> =
            StdArc::new(MemInboxSource::new(vec![], vec![], vec![]));
        client.connect(source2).expect("connect again");
        assert!(client.is_connected());
        client.close().expect("close");
        assert!(!client.is_connected());
        // 幂等 close。
        client.close().expect("close again");
        assert!(!client.is_connected());
        drop(effects);
    }

    #[tokio::test]
    async fn fetch_inbox_returns_unread_and_marks_read() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let mem = StdArc::new(MemInboxSource::new(
            vec![msg("m1", "alice", "hello")],
            vec![msg("m2", "leader", "all hands")],
            vec![task("t1", "pending"), task("t2", "completed")],
        ));
        let source: Arc<dyn ExternalInboxSource> = mem.clone();
        client.connect(source).expect("connect");

        let view = client.fetch_inbox(true).await.expect("fetch");
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.tasks.len(), 2);
        assert!(!view.is_empty());
        // mark_read=true → 数据源标记了 m1/m2。
        assert_eq!(mem.read.lock().unwrap().len(), 2);
        drop(effects);
    }

    #[tokio::test]
    async fn read_inbox_composes_messages_and_board() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let source: Arc<dyn ExternalInboxSource> = StdArc::new(MemInboxSource::new(
            vec![msg("m1", "alice", "hello")],
            vec![],
            vec![task("t1", "pending")],
        ));
        client.connect(source).expect("connect");

        let text = client.read_inbox(false).await.expect("read_inbox");
        assert!(text.contains("hello"), "{text}");
        assert!(text.contains("Title t1"), "{text}");
        assert!(text.contains("<team-inbound"), "{text}");
        assert!(text.contains("<team-event"), "{text}");
        assert!(text.contains("\n\n"), "消息与看板空行分隔");
        drop(effects);
    }

    #[tokio::test]
    async fn read_inbox_empty_falls_back() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let source: Arc<dyn ExternalInboxSource> =
            StdArc::new(MemInboxSource::new(vec![], vec![], vec![]));
        client.connect(source).expect("connect");
        assert_eq!(
            client.read_inbox(false).await.expect("read"),
            "(inbox empty)"
        );
        drop(effects);
    }

    #[tokio::test]
    async fn not_connected_errors_explicitly() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        let err = client.fetch_inbox(true).await.unwrap_err();
        assert!(err.0.contains("not connected"), "{err}");
        let err2 = client.read_inbox(true).await.unwrap_err();
        assert!(err2.0.contains("not connected"), "{err2}");
        let err3 = client.watch(&NoopObserver).await.unwrap_err();
        assert!(err3.0.contains("not connected"), "{err3}");
        drop(effects);
    }

    struct NoopObserver;

    #[async_trait]
    impl InboxObserver for NoopObserver {
        async fn on_inbox(&self, _view: InboxView) {}
    }

    #[tokio::test]
    async fn read_inbox_expands_scheduler_template() {
        let (ctx, effects) = build_ctx();
        let factory = ctx
            .service::<dyn ExternalTeamClientFactory>(&EXTERNAL_CLIENT)
            .expect("external-client seam");
        let client = factory.build(&descriptor());
        // 框架模板消息:scheduler_task_start,meta 引用任务行。
        let templated = InboxMessage {
            view: MessageView {
                broadcast: false,
                timestamp: 1_700_000_000_000,
                from_member_name: "leader".to_string(),
                message_id: "m-tpl".to_string(),
                content: String::new(),
            },
            meta: Some(r#"{"template":"scheduler_task_start","refs":{"task":"t-9"}}"#.to_string()),
        };
        let mem = StdArc::new(TemplatedSource {
            templated: vec![templated],
        });
        let source: Arc<dyn ExternalInboxSource> = mem.clone();
        client.connect(source).expect("connect");
        let text = client.read_inbox(false).await.expect("read_inbox");
        assert!(text.contains("[任务开工]"), "{text}");
        assert!(text.contains("Build the widget"), "{text}");
        drop(effects);
    }

    /// 带任务行读取的内存源(供模板展开)。
    struct TemplatedSource {
        templated: Vec<InboxMessage>,
    }

    impl Seam for TemplatedSource {}

    #[async_trait]
    impl ExternalInboxSource for TemplatedSource {
        async fn unread_direct_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            self.templated.clone()
        }

        async fn unread_broadcast_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            vec![]
        }

        async fn mark_message_read(&self, _message_id: &str, _member_name: &str) {}

        async fn list_tasks(&self) -> Vec<TaskLineView> {
            vec![]
        }

        async fn get_task(&self, task_id: &str) -> Option<ah_contracts::team_message::TaskView> {
            if task_id == "t-9" {
                Some(ah_contracts::team_message::TaskView {
                    task_id: "t-9".to_string(),
                    title: "Build the widget".to_string(),
                    content: "Do it".to_string(),
                    status: "in_progress".to_string(),
                    assignee: "dev-1".to_string(),
                    reviewers: vec!["bob".to_string()],
                    review_round: Some(1),
                    max_review_rounds: Some(3),
                })
            } else {
                None
            }
        }

        async fn get_member(
            &self,
            _member_name: &str,
        ) -> Option<ah_contracts::team_message::MemberView> {
            None
        }
    }
}
