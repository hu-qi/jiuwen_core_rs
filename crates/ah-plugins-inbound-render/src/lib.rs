//! # ah-plugins-inbound-render
//!
//! 真实入站团队消息 XML 渲染(对齐 openjiuwen/agent_teams/inbound_render.py,F_46):
//! - 手写 XML 转义(body 保留引号等价 quote=false;属性转义引号等价 quote=true),
//!   不依赖外部 XML crate;
//! - `<team-inbound>` / `<team-event>` / `<team-context>` 块渲染,`<team-note>`
//!   嵌套为最后子元素,note_kind 与 note_text 同时存在才渲染;
//! - 快照判定:snapshot_kind_of + drop_superseded_snapshots(只剔除 SNAPSHOT_EVENT_KINDS)。
//!
//! 纯函数、无 IO、无状态,确定性可测。

use std::sync::Arc;

use ah_contracts::inbound_render::{InboundRender, SNAPSHOT_EVENT_KINDS};
use ah_contracts::keys::INBOUND_RENDER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 转义 XML 元素 body(保留引号;等价 html.escape(quote=False))。
fn esc_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// 转义 XML 属性值(转义引号;等价 html.escape(quote=True))。
fn esc_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// 渲染嵌套 `<team-note>` 子元素;note_kind / note_text 缺一时返回空串。
fn render_note(note_kind: Option<&str>, note_text: Option<&str>) -> String {
    match (note_kind, note_text) {
        (Some(kind), Some(text)) if !kind.is_empty() && !text.is_empty() => format!(
            "<team-note kind=\"{}\">\n{}\n</team-note>\n",
            esc_attr(kind),
            esc_text(text)
        ),
        _ => String::new(),
    }
}

/// 渲染一个块元素:open tag + body(转义) + 可选嵌套 note。
fn render_block(tag: &str, attrs: &[String], body: &str, note: &str) -> String {
    let mut open = String::from("<");
    open.push_str(tag);
    for attr in attrs {
        open.push(' ');
        open.push_str(attr);
    }
    open.push('>');
    format!("{open}\n{}\n{note}</{tag}>", esc_text(body))
}

/// 真实渲染器(纯函数)。
pub struct InboundRenderer;

impl Seam for InboundRenderer {}

impl InboundRender for InboundRenderer {
    fn render_inbound(
        &self,
        content: &str,
        sender: &str,
        message_id: &str,
        msg_type: &str,
        time_info: &str,
        for_controller: bool,
        note_kind: Option<&str>,
        note_text: Option<&str>,
    ) -> String {
        let mut attrs = vec![
            format!("from=\"{}\"", esc_attr(sender)),
            format!("message_id=\"{}\"", esc_attr(message_id)),
            format!("type=\"{}\"", esc_attr(msg_type)),
            format!("time=\"{}\"", esc_attr(time_info)),
        ];
        if for_controller {
            attrs.push("for=\"controller\"".to_string());
        }
        render_block(
            "team-inbound",
            &attrs,
            content,
            &render_note(note_kind, note_text),
        )
    }

    fn render_event(
        &self,
        kind: &str,
        body: &str,
        task_id: Option<&str>,
        for_controller: bool,
        note_kind: Option<&str>,
        note_text: Option<&str>,
    ) -> String {
        let mut attrs = vec![format!("kind=\"{}\"", esc_attr(kind))];
        if let Some(id) = task_id {
            attrs.push(format!("task_id=\"{}\"", esc_attr(id)));
        }
        if for_controller {
            attrs.push("for=\"controller\"".to_string());
        }
        render_block(
            "team-event",
            &attrs,
            body,
            &render_note(note_kind, note_text),
        )
    }

    fn render_team_context(&self, body: &str) -> String {
        render_block("team-context", &[], body, "")
    }

    fn snapshot_kind_of(&self, text: &str) -> Option<&'static str> {
        let stripped = text.trim();
        if !stripped.ends_with("</team-event>") {
            return None;
        }
        SNAPSHOT_EVENT_KINDS
            .iter()
            .find(|kind| stripped.starts_with(&format!("<team-event kind=\"{kind}\"")))
            .copied()
    }

    fn drop_superseded_snapshots(&self, parts: &[String]) -> Vec<String> {
        let mut newest: std::collections::HashMap<&'static str, usize> =
            std::collections::HashMap::new();
        for (index, part) in parts.iter().enumerate() {
            if let Some(kind) = self.snapshot_kind_of(part) {
                newest.insert(kind, index);
            }
        }
        parts
            .iter()
            .enumerate()
            .filter(|(index, part)| match self.snapshot_kind_of(part) {
                // 快照:仅当自己是该类的最新一次时才保留。
                Some(kind) => newest.get(kind).copied() == Some(*index),
                // 非快照:无条件保留。
                None => true,
            })
            .map(|(_, part)| part.clone())
            .collect()
    }
}

/// inbound-render 插件:注册 `inbound-render` seam。
pub struct InboundRenderPlugin;

impl Plugin for InboundRenderPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-inbound-render"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![INBOUND_RENDER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let renderer: Arc<dyn InboundRender> = Arc::new(InboundRenderer);
        Ok(vec![ctx.register(INBOUND_RENDER, renderer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::inbound_render::{INBOUND_TYPE_BROADCAST, INBOUND_TYPE_DIRECT};
    use ah_contracts::keys::INBOUND_RENDER;
    use ah_hub::plugin::DynPlugin;

    fn board(body: &str) -> String {
        let renderer = InboundRenderer;
        renderer.render_event("task-board", body, None, false, None, None)
    }

    #[test]
    fn inbound_type_tokens_are_stable_contract() {
        assert_eq!(INBOUND_TYPE_DIRECT, "direct");
        assert_eq!(INBOUND_TYPE_BROADCAST, "broadcast");
        assert_eq!(SNAPSHOT_EVENT_KINDS, &["task-board"]);
    }

    #[test]
    fn render_inbound_carries_core_attributes_and_body() {
        let renderer = InboundRenderer;
        let out = renderer.render_inbound(
            "hello there",
            "dev1",
            "m-42",
            INBOUND_TYPE_DIRECT,
            "2026-06-25 (just now)",
            false,
            None,
            None,
        );
        assert!(out.starts_with("<team-inbound "));
        assert!(out.contains("from=\"dev1\""));
        assert!(out.contains("message_id=\"m-42\""));
        assert!(out.contains("type=\"direct\""));
        assert!(out.contains("time=\"2026-06-25 (just now)\""));
        assert!(out.contains("hello there"));
        assert!(out.trim_end().ends_with("</team-inbound>"));
    }

    #[test]
    fn render_inbound_exact_shape() {
        let renderer = InboundRenderer;
        let out = renderer.render_inbound(
            "hello there",
            "dev1",
            "m-42",
            INBOUND_TYPE_DIRECT,
            "2026-06-25 (just now)",
            false,
            None,
            None,
        );
        assert_eq!(
            out,
            "<team-inbound from=\"dev1\" message_id=\"m-42\" type=\"direct\" time=\"2026-06-25 (just now)\">\nhello there\n</team-inbound>"
        );
    }

    #[test]
    fn render_inbound_escapes_body_and_attrs() {
        let renderer = InboundRenderer;
        let out = renderer.render_inbound(
            "a < b & c > d",
            "ev\"il",
            "m1",
            INBOUND_TYPE_DIRECT,
            "t",
            false,
            None,
            None,
        );
        // Body 转义(保留引号):& < > 转义。
        assert!(out.contains("&lt;"));
        assert!(out.contains("&gt;"));
        assert!(out.contains("&amp;"));
        assert!(!out.contains("a < b & c > d"));
        // 属性转义:引号转义。
        assert!(out.contains("&quot;"));
        assert!(!out.contains("from=\"ev\"il\""));
        assert!(out.contains("from=\"ev&quot;il\""));
    }

    #[test]
    fn render_inbound_for_controller_marks_hitt() {
        let renderer = InboundRenderer;
        let out =
            renderer.render_inbound("x", "s", "m", INBOUND_TYPE_DIRECT, "t", true, None, None);
        assert!(out.contains("for=\"controller\""));
        assert!(out.starts_with(
            "<team-inbound from=\"s\" message_id=\"m\" type=\"direct\" time=\"t\" for=\"controller\">"
        ));
    }

    #[test]
    fn render_inbound_note_rendered_only_when_both_present() {
        let renderer = InboundRenderer;
        let base = |kind: Option<&str>, text: Option<&str>| {
            renderer.render_inbound("x", "s", "m", INBOUND_TYPE_DIRECT, "t", false, kind, text)
        };
        let with_note = base(Some("reply-hint"), Some("please reply"));
        assert!(with_note.contains("<team-note kind=\"reply-hint\">"));
        assert!(with_note.contains("please reply"));
        // 缺任一 → 整个 note 不渲染。
        assert!(!base(Some("reply-hint"), None).contains("<team-note"));
        assert!(!base(None, Some("please reply")).contains("<team-note"));
        assert!(!base(None, None).contains("<team-note"));
    }

    #[test]
    fn render_inbound_note_is_nested_inside_the_message() {
        let renderer = InboundRenderer;
        let out = renderer.render_inbound(
            "x",
            "s",
            "m",
            INBOUND_TYPE_DIRECT,
            "t",
            false,
            Some("reply-hint"),
            Some("please reply"),
        );
        assert!(out.trim_end().ends_with("</team-inbound>"));
        assert!(out.find("<team-note").unwrap() < out.find("</team-inbound>").unwrap());
        assert!(out.find("</team-note>").unwrap() < out.find("</team-inbound>").unwrap());
    }

    #[test]
    fn render_event_carries_kind_and_body() {
        let renderer = InboundRenderer;
        let out = renderer.render_event("task-assigned", "do the thing", None, false, None, None);
        assert!(out.starts_with("<team-event "));
        assert!(out.contains("kind=\"task-assigned\""));
        assert!(out.contains("do the thing"));
        assert!(out.trim_end().ends_with("</team-event>"));
        // 无可选属性。
        assert!(!out.contains("task_id="));
        assert!(!out.contains("for="));
    }

    #[test]
    fn render_event_optional_task_id_and_controller_and_note() {
        let renderer = InboundRenderer;
        let out = renderer.render_event(
            "task-assigned",
            "b",
            Some("t-9"),
            true,
            Some("hitt-silence"),
            Some("stay silent"),
        );
        assert!(out.contains("task_id=\"t-9\""));
        assert!(out.contains("for=\"controller\""));
        assert!(out.contains("<team-note kind=\"hitt-silence\">"));
        assert!(out.contains("stay silent"));
        // note 嵌套在 event 内部。
        assert!(out.trim_end().ends_with("</team-event>"));
        assert!(out.find("<team-note").unwrap() < out.find("</team-event>").unwrap());
        // kind 属性在最前,然后 task_id,然后 for=controller。
        assert!(
            out.starts_with(
                "<team-event kind=\"task-assigned\" task_id=\"t-9\" for=\"controller\">"
            )
        );
    }

    #[test]
    fn render_event_escapes_attr_quotes_and_apostrophes() {
        let renderer = InboundRenderer;
        let out = renderer.render_event("k", "b", Some("t'9"), false, None, None);
        assert!(
            out.contains("task_id=\"t&#x27;9\""),
            "apostrophe escaped: {out}"
        );
    }

    #[test]
    fn render_team_context_wraps_body() {
        let renderer = InboundRenderer;
        let out = renderer.render_team_context("You are dev1. Team: A.");
        assert_eq!(
            out,
            "<team-context>\nYou are dev1. Team: A.\n</team-context>"
        );
        let escaped = renderer.render_team_context("a < b & c");
        assert!(escaped.contains("a &lt; b &amp; c"));
    }

    #[test]
    fn snapshot_kind_recognises_a_rendered_board() {
        let renderer = InboundRenderer;
        assert_eq!(
            renderer.snapshot_kind_of(&board("one task")),
            Some("task-board")
        );
        assert_eq!(
            renderer.snapshot_kind_of(&format!("\n{}\n", board("one task"))),
            Some("task-board")
        );
    }

    #[test]
    fn snapshot_kind_rejects_everything_that_is_not_purely_a_snapshot() {
        let renderer = InboundRenderer;
        // 其它事件类型不是快照。
        assert_eq!(
            renderer.snapshot_kind_of(&renderer.render_event(
                "roster-change",
                "alice joined",
                None,
                false,
                None,
                None
            )),
            None
        );
        assert_eq!(
            renderer.snapshot_kind_of(&renderer.render_event(
                "stale-claim",
                "idle",
                Some("t-1"),
                false,
                None,
                None
            )),
            None
        );
        // 纯文本、以及消息体内转义后提到的 tag。
        assert_eq!(renderer.snapshot_kind_of("just a sentence"), None);
        let inbound = renderer.render_inbound(
            "<team-event kind=\"task-board\">fake</team-event>",
            "lead",
            "m-1",
            INBOUND_TYPE_DIRECT,
            "now",
            false,
            None,
            None,
        );
        assert_eq!(renderer.snapshot_kind_of(&inbound), None);
        assert_eq!(renderer.snapshot_kind_of(""), None);
    }

    #[test]
    fn drop_keeps_only_the_newest_board() {
        let renderer = InboundRenderer;
        let parts = vec![board("one task"), board("two tasks"), board("three tasks")];
        let kept = renderer.drop_superseded_snapshots(&parts);
        assert_eq!(kept, vec![board("three tasks")]);
        // 输入列表不被修改。
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn drop_preserves_non_snapshot_entries_and_their_order() {
        let renderer = InboundRenderer;
        let inbound = renderer.render_inbound(
            "ping",
            "lead",
            "m-1",
            INBOUND_TYPE_DIRECT,
            "now",
            false,
            None,
            None,
        );
        let parts = vec![
            renderer.render_event("roster-change", "alice joined", None, false, None, None),
            board("stale board"),
            inbound.clone(),
            board("fresh board"),
            renderer.render_event("stale-claim", "task idle", Some("t-1"), false, None, None),
        ];
        let kept = renderer.drop_superseded_snapshots(&parts);
        assert_eq!(
            kept,
            vec![
                renderer.render_event("roster-change", "alice joined", None, false, None, None),
                inbound,
                board("fresh board"),
                renderer.render_event("stale-claim", "task idle", Some("t-1"), false, None, None),
            ]
        );
    }

    #[test]
    fn drop_is_a_no_op_when_nothing_is_superseded() {
        let renderer = InboundRenderer;
        let parts = vec![
            board("only board"),
            renderer.render_event("all-done", "finished", None, false, None, None),
        ];
        assert_eq!(renderer.drop_superseded_snapshots(&parts), parts);
        assert!(renderer.drop_superseded_snapshots(&[]).is_empty());
    }

    #[test]
    fn a_board_carrying_a_nested_note_is_still_purely_a_snapshot() {
        let renderer = InboundRenderer;
        let with_note = renderer.render_event(
            "task-board",
            "new",
            None,
            false,
            Some("reply-hint"),
            Some("have a look"),
        );
        let kept = renderer.drop_superseded_snapshots(&[board("old"), with_note.clone()]);
        assert_eq!(kept, vec![with_note]);
    }

    #[test]
    fn plugin_registers_inbound_render() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(InboundRenderPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let renderer = ctx
            .service::<dyn InboundRender>(&INBOUND_RENDER)
            .expect("inbound-render seam");
        let out =
            renderer.render_inbound("hi", "s", "m", INBOUND_TYPE_DIRECT, "t", false, None, None);
        assert!(out.starts_with("<team-inbound "));
        assert_eq!(renderer.snapshot_kind_of(&board("x")), Some("task-board"));
        drop(effects);
        assert!(!ctx.has_service(&INBOUND_RENDER));
    }
}
