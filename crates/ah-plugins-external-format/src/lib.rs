//! # ah-plugins-external-format
//!
//! Real external CLI inbound rendering (aligned with openjiuwen/agent_teams/
//! external/format.py): composes the inbound-render and timefmt seams into
//! the exact <team-inbound>/<team-event kind="task-board"> XML the in-process
//! dispatcher produces, so external CLI members read the same input shape.

use std::collections::HashMap;
use std::sync::Arc;

use ah_contracts::external_format::{
    ExternalFormat, MessageView, TERMINAL_TASK_STATUSES, TaskLineView,
};
use ah_contracts::inbound_render::InboundRender;
use ah_contracts::keys::EXTERNAL_FORMAT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::timefmt::Timefmt;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// Real composition renderer (pure functions, dependency-injected seams).
pub struct ExternalFormatEngine;

impl Seam for ExternalFormatEngine {}

impl ExternalFormat for ExternalFormatEngine {
    fn render_message(
        &self,
        message: &MessageView,
        is_human_agent: bool,
        now_ms: i64,
        body: Option<&str>,
        reply_hint: Option<&str>,
        hitt_silence_note: Option<&str>,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String {
        let msg_type = if message.broadcast {
            "broadcast"
        } else {
            "direct"
        };
        let time_info = timefmt.format_time_context(Some(message.timestamp), now_ms);
        let content = body.unwrap_or(&message.content);
        if is_human_agent {
            return render.render_inbound(
                content,
                &message.from_member_name,
                &message.message_id,
                msg_type,
                &time_info,
                true,
                Some("hitt-silence"),
                hitt_silence_note,
            );
        }
        if body.is_some() {
            // Framework template message: no reply hint.
            return render.render_inbound(
                content,
                &message.from_member_name,
                &message.message_id,
                msg_type,
                &time_info,
                false,
                None,
                None,
            );
        }
        render.render_inbound(
            content,
            &message.from_member_name,
            &message.message_id,
            msg_type,
            &time_info,
            false,
            Some("reply-hint"),
            reply_hint,
        )
    }

    fn render_messages(
        &self,
        messages: &[MessageView],
        is_human_agent: bool,
        now_ms: i64,
        bodies: Option<&HashMap<String, String>>,
        reply_hint: Option<&str>,
        hitt_silence_note: Option<&str>,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String {
        let bodies = bodies.cloned().unwrap_or_default();
        let rendered: Vec<String> = messages
            .iter()
            .map(|m| {
                self.render_message(
                    m,
                    is_human_agent,
                    now_ms,
                    bodies.get(&m.message_id).map(|s| s.as_str()),
                    reply_hint,
                    hitt_silence_note,
                    render,
                    timefmt,
                )
            })
            .collect();
        rendered.join("\n\n")
    }

    fn render_task_line(
        &self,
        task: &TaskLineView,
        now_ms: i64,
        unassigned_marker: &str,
        timefmt: &dyn Timefmt,
    ) -> String {
        let assignee = match &task.assignee {
            Some(name) if !name.is_empty() => format!(" → {name}"),
            _ => unassigned_marker.to_string(),
        };
        let time_info = timefmt.format_time_context(task.updated_at, now_ms);
        format!(
            "- [{}] [{}] {}: {}{assignee} ({time_info})",
            task.task_id, task.status, task.title, task.content,
        )
    }

    fn render_task_board(
        &self,
        tasks: &[TaskLineView],
        is_leader: bool,
        now_ms: i64,
        leader_header: &str,
        teammate_header: &str,
        unassigned_marker: &str,
        render: &dyn InboundRender,
        timefmt: &dyn Timefmt,
    ) -> String {
        let incomplete: Vec<&TaskLineView> = tasks
            .iter()
            .filter(|t| !TERMINAL_TASK_STATUSES.contains(&t.status.as_str()))
            .collect();
        if incomplete.is_empty() {
            return String::new();
        }
        let header = if is_leader {
            leader_header
        } else {
            teammate_header
        };
        let mut lines = vec![header.to_string()];
        for task in incomplete {
            lines.push(self.render_task_line(task, now_ms, unassigned_marker, timefmt));
        }
        render.render_event("task-board", &lines.join("\n"), None, false, None, None)
    }
}

/// external-format 插件:注册 `external-format` seam。
pub struct ExternalFormatPlugin;

impl Plugin for ExternalFormatPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-external-format"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![EXTERNAL_FORMAT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let engine: Arc<dyn ExternalFormat> = Arc::new(ExternalFormatEngine);
        Ok(vec![ctx.register(EXTERNAL_FORMAT, engine)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::EXTERNAL_FORMAT;
    use ah_hub::plugin::DynPlugin;
    use ah_plugins_inbound_render::InboundRenderer;
    use ah_plugins_timefmt::TimefmtService;

    fn msg(broadcast: bool, from: &str, id: &str, content: &str) -> MessageView {
        MessageView {
            broadcast,
            timestamp: 1_700_000_000_000,
            from_member_name: from.to_string(),
            message_id: id.to_string(),
            content: content.to_string(),
        }
    }

    fn task(id: &str, status: &str, assignee: Option<&str>) -> TaskLineView {
        TaskLineView {
            task_id: id.to_string(),
            title: format!("Title {id}"),
            content: "body".to_string(),
            status: status.to_string(),
            assignee: assignee.map(|s| s.to_string()),
            updated_at: Some(1_700_000_000_000),
        }
    }

    fn engine() -> (ExternalFormatEngine, InboundRenderer, TimefmtService) {
        (
            ExternalFormatEngine,
            InboundRenderer,
            TimefmtService::with_offset(0),
        )
    }

    #[test]
    fn render_ordinary_message_with_reply_hint() {
        let (e, render, tf) = engine();
        let m = msg(false, "alice", "m1", "hello");
        let out = e.render_message(
            &m,
            false,
            1_700_000_000_100,
            None,
            Some("please reply"),
            Some("silence"),
            &render,
            &tf,
        );
        assert!(
            out.starts_with("<team-inbound from=\"alice\" message_id=\"m1\" type=\"direct\""),
            "{out}"
        );
        assert!(out.contains("hello"));
        assert!(out.contains("<team-note kind=\"reply-hint\">"), "{out}");
        assert!(out.contains("please reply"));
        assert!(!out.contains("hitt-silence"));
        // broadcast type.
        let b = msg(true, "leader", "m2", "all hands");
        let out2 = e.render_message(
            &b,
            false,
            1_700_000_000_100,
            None,
            Some("hint"),
            None,
            &render,
            &tf,
        );
        assert!(out2.contains("type=\"broadcast\""));
    }

    #[test]
    fn render_human_agent_message_with_controller_note() {
        let (e, render, tf) = engine();
        let m = msg(false, "human_agent", "m3", "for controller");
        let out = e.render_message(
            &m,
            true,
            1_700_000_000_100,
            None,
            Some("reply"),
            Some("stay silent"),
            &render,
            &tf,
        );
        assert!(out.contains("for=\"controller\""), "{out}");
        assert!(out.contains("<team-note kind=\"hitt-silence\">"));
        assert!(out.contains("stay silent"));
        assert!(!out.contains("reply-hint"), "human agent 不用 reply-hint");
    }

    #[test]
    fn render_template_body_drops_reply_hint() {
        let (e, render, tf) = engine();
        let m = msg(false, "leader", "m4", "stale content");
        let out = e.render_message(
            &m,
            false,
            1_700_000_000_100,
            Some("framework body"),
            Some("hint"),
            None,
            &render,
            &tf,
        );
        assert!(out.contains("framework body"));
        assert!(!out.contains("stale content"), "body 优先");
        assert!(!out.contains("reply-hint"), "框架模板消息无 reply hint");
    }

    #[test]
    fn render_messages_batch_with_bodies() {
        let (e, render, tf) = engine();
        let msgs = vec![msg(false, "a", "m1", "one"), msg(false, "b", "m2", "two")];
        let bodies = HashMap::from([("m1".to_string(), "templated".to_string())]);
        let out = e.render_messages(
            &msgs,
            false,
            1_700_000_000_100,
            Some(&bodies),
            Some("h"),
            None,
            &render,
            &tf,
        );
        assert!(out.contains("templated"));
        assert!(out.contains("two"));
        assert!(out.contains("\n\n"), "消息间空行分隔");
    }

    #[test]
    fn render_task_line_with_and_without_assignee() {
        let (e, _render, tf) = engine();
        let t = task("t1", "in_progress", Some("dev-1"));
        let line = e.render_task_line(&t, 1_700_000_000_100, " (待领取)", &tf);
        assert!(
            line.contains("- [t1] [in_progress] Title t1: body → dev-1"),
            "{line}"
        );
        let un = task("t2", "pending", None);
        let line2 = e.render_task_line(&un, 1_700_000_000_100, " (待领取)", &tf);
        assert!(line2.contains("(待领取)"), "{line2}");
    }

    #[test]
    fn render_task_board_filters_terminal_and_role_header() {
        let (e, render, tf) = engine();
        let tasks = vec![
            task("t1", "pending", None),
            task("t2", "completed", Some("a")),
            task("t3", "cancelled", Some("b")),
        ];
        let out = e.render_task_board(
            &tasks,
            true,
            1_700_000_000_100,
            "LEADER-BOARD",
            "TEAMMATE-LIST",
            " (待领取)",
            &render,
            &tf,
        );
        assert!(out.starts_with("<team-event kind=\"task-board\""), "{out}");
        assert!(out.contains("LEADER-BOARD"));
        assert!(out.contains("- [t1]"), "pending 保留");
        assert!(!out.contains("[t2]"), "completed 过滤");
        assert!(!out.contains("[t3]"), "cancelled 过滤");
        // teammate header.
        let out2 = e.render_task_board(
            &tasks,
            false,
            1_700_000_000_100,
            "LEADER-BOARD",
            "TEAMMATE-LIST",
            " (待领取)",
            &render,
            &tf,
        );
        assert!(out2.contains("TEAMMATE-LIST"));
        // 空行动项 → 空串。
        let done_only = vec![task("t9", "completed", None)];
        assert!(
            e.render_task_board(
                &done_only,
                true,
                1_700_000_000_100,
                "H",
                "T",
                " (待领取)",
                &render,
                &tf
            )
            .is_empty()
        );
    }

    #[test]
    fn plugin_registers_external_format() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ExternalFormatPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let engine = ctx
            .service::<dyn ExternalFormat>(&EXTERNAL_FORMAT)
            .expect("external-format seam");
        let _ = engine;
        drop(effects);
        assert!(!ctx.has_service(&EXTERNAL_FORMAT));
    }
}
