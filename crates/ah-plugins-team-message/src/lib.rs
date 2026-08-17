//! # ah-plugins-team-message
//!
//! Real two-phase framework-templated team message rendering (aligned with
//! openjiuwen/agent_teams/message_template.py, F_63):
//! - send stores intent (meta = {template, refs, params}); delivery renders
//!   against the current task/member rows;
//! - placeholder contract: single-pass {{ns.field}} substitution (values are
//!   never rescanned), explicit field whitelists, unknown ns/field/missing
//!   row renders <missing:ns.field>;
//! - missing referenced row -> RefUnresolved -> fallback line;
//! - ordinary messages (no/malformed/no-template meta) pass through verbatim.

use std::collections::BTreeMap;
use std::sync::Arc;

use ah_contracts::keys::TEAM_MESSAGE;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_message::{ExpandedMessage, MemberView, MessageMeta, TaskView, TeamMessage};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

fn is_name_char(c: char) -> bool {
    c.is_ascii_lowercase() || c == '_'
}

/// Find the next {{ns.field}} (lowercase+underscore names) in text[start..].
/// Returns (placeholder_start, end_after_}}, ns, field).
fn find_placeholder(text: &str, start: usize) -> Option<(usize, usize, String, String)> {
    let bytes = text.as_bytes();
    let mut i = start;
    while i + 2 <= bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let mut j = i + 2;
            // Skip whitespace.
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
                j += 1;
            }
            let ns_start = j;
            while j < bytes.len() && is_name_char(bytes[j] as char) {
                j += 1;
            }
            let ns: String = text[ns_start..j].to_string();
            if ns.is_empty() || j >= bytes.len() || bytes[j] != b'.' {
                i += 2;
                continue;
            }
            j += 1;
            let field_start = j;
            while j < bytes.len() && is_name_char(bytes[j] as char) {
                j += 1;
            }
            let field: String = text[field_start..j].to_string();
            if field.is_empty() {
                i += 2;
                continue;
            }
            // Skip whitespace.
            while j < bytes.len() && (bytes[j] == b' ' || bytes[j] == b'\t' || bytes[j] == b'\n') {
                j += 1;
            }
            if j + 1 >= bytes.len() || bytes[j] != b'}' || bytes[j + 1] != b'}' {
                i += 2;
                continue;
            }
            return Some((i, j + 2, ns, field));
        }
        i += 1;
    }
    None
}

/// Task projection field reader.
fn task_field(task: &TaskView, field: &str) -> Option<String> {
    match field {
        "task_id" => Some(task.task_id.clone()),
        "title" => Some(task.title.clone()),
        "content" => Some(task.content.clone()),
        "status" => Some(task.status.clone()),
        "assignee" => Some(task.assignee.clone()),
        "reviewer" => Some(task.reviewers.join(", ")),
        "review_round" => task.review_round.map(|v| v.to_string()),
        "max_review_rounds" => task.max_review_rounds.map(|v| v.to_string()),
        _ => None,
    }
}

/// Member projection field reader.
fn member_field(member: &MemberView, field: &str) -> Option<String> {
    match field {
        "member_name" => Some(member.member_name.clone()),
        "display_name" => Some(member.display_name.clone()),
        "desc" => Some(member.desc.clone()),
        _ => None,
    }
}

/// Real two-phase rendering implementation.
pub struct MessageTemplateEngine;

impl Seam for MessageTemplateEngine {}

impl TeamMessage for MessageTemplateEngine {
    fn parse_meta(&self, raw: Option<&str>) -> Option<MessageMeta> {
        let raw = raw?;
        if raw.trim().is_empty() {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        let obj = value.as_object()?;
        let template = obj.get("template")?.as_str()?;
        if template.is_empty() {
            return None;
        }
        let refs = obj
            .get("refs")
            .and_then(|r| r.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let params = obj
            .get("params")
            .and_then(|p| p.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Some(MessageMeta {
            template: template.to_string(),
            refs,
            params,
        })
    }

    fn build_meta(
        &self,
        template: &str,
        refs: Option<&[(String, String)]>,
        params: Option<&[(String, String)]>,
    ) -> MessageMeta {
        MessageMeta {
            template: template.to_string(),
            refs: refs
                .map(|r| r.iter().cloned().collect())
                .unwrap_or_default(),
            params: params
                .map(|p| p.iter().cloned().collect())
                .unwrap_or_default(),
        }
    }

    fn fallback_line(&self, meta: &MessageMeta) -> String {
        let template = if meta.template.is_empty() {
            "?".to_string()
        } else {
            meta.template.clone()
        };
        let task_id = meta.refs.get("task").cloned().unwrap_or_default();
        if task_id.is_empty() {
            format!("[{template}] — details unavailable.")
        } else {
            format!(
                "[{template}] task_id={task_id} — details unavailable, call view_task for the task."
            )
        }
    }

    fn substitute(
        &self,
        template: &str,
        task: Option<&TaskView>,
        member: Option<&MemberView>,
        params: &BTreeMap<String, String>,
    ) -> String {
        let mut out = String::new();
        let mut cursor = 0usize;
        while let Some((start, end, ns, field)) = find_placeholder(template, cursor) {
            out.push_str(&template[cursor..start]);
            let resolved = match ns.as_str() {
                "task" => task.and_then(|t| task_field(t, &field)),
                "member" => member.and_then(|m| member_field(m, &field)),
                "param" => params.get(&field).cloned(),
                _ => None,
            };
            match resolved {
                Some(value) => out.push_str(&value),
                None => out.push_str(&format!("<missing:{ns}.{field}>")),
            }
            cursor = end;
        }
        out.push_str(&template[cursor..]);
        out
    }

    fn expand(
        &self,
        content: &str,
        meta_raw: Option<&str>,
        template_content: &str,
        task: Option<&TaskView>,
        member: Option<&MemberView>,
    ) -> ExpandedMessage {
        let Some(meta) = self.parse_meta(meta_raw) else {
            return ExpandedMessage {
                body: content.to_string(),
                is_template: false,
            };
        };
        // refs 引用的行必须存在,否则降级 fallback。
        if meta.refs.contains_key("task") && task.is_none() {
            return ExpandedMessage {
                body: self.fallback_line(&meta),
                is_template: true,
            };
        }
        if meta.refs.contains_key("member") && member.is_none() {
            return ExpandedMessage {
                body: self.fallback_line(&meta),
                is_template: true,
            };
        }
        let body = self.substitute(template_content, task, member, &meta.params);
        ExpandedMessage {
            body,
            is_template: true,
        }
    }
}

/// team-message plugin: registers the `team-message` seam.
pub struct TeamMessagePlugin;

impl Plugin for TeamMessagePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-message"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_MESSAGE]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let engine: Arc<dyn TeamMessage> = Arc::new(MessageTemplateEngine);
        Ok(vec![ctx.register(TEAM_MESSAGE, engine)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TEAM_MESSAGE;
    use ah_hub::plugin::DynPlugin;
    use std::collections::BTreeMap;

    fn task() -> TaskView {
        TaskView {
            task_id: "t-1".to_string(),
            title: "Build the widget".to_string(),
            content: "Do the thing".to_string(),
            status: "in_progress".to_string(),
            assignee: "alice".to_string(),
            reviewers: vec!["bob".to_string(), "carol".to_string()],
            review_round: Some(2),
            max_review_rounds: Some(3),
        }
    }

    fn member() -> MemberView {
        MemberView {
            member_name: "alice".to_string(),
            display_name: "Alice".to_string(),
            desc: "builder".to_string(),
        }
    }

    #[test]
    fn parse_meta_accepts_and_rejects() {
        let engine = MessageTemplateEngine;
        // 普通消息。
        assert!(engine.parse_meta(None).is_none());
        assert!(engine.parse_meta(Some("")).is_none());
        assert!(engine.parse_meta(Some("not json")).is_none());
        assert!(
            engine.parse_meta(Some(r#"{"foo": 1}"#)).is_none(),
            "无 template 键"
        );
        assert!(
            engine.parse_meta(Some(r#"{"template": ""}"#)).is_none(),
            "空 template"
        );
        // 合法 meta。
        let meta = engine
            .parse_meta(Some(r#"{"template":"scheduler.handoff","refs":{"task":"t-1"},"params":{"reason":"fail"}}"#))
            .expect("meta");
        assert_eq!(meta.template, "scheduler.handoff");
        assert_eq!(meta.refs.get("task").map(|s| s.as_str()), Some("t-1"));
        assert_eq!(meta.params.get("reason").map(|s| s.as_str()), Some("fail"));
    }

    #[test]
    fn build_meta_assembles_payload() {
        let engine = MessageTemplateEngine;
        let meta = engine.build_meta(
            "scheduler.handoff",
            Some(&[("task".to_string(), "t-9".to_string())]),
            Some(&[("feedback".to_string(), "needs work".to_string())]),
        );
        assert_eq!(meta.template, "scheduler.handoff");
        assert_eq!(meta.refs.get("task").unwrap(), "t-9");
        assert_eq!(meta.params.get("feedback").unwrap(), "needs work");
        // 无 refs/params。
        let bare = engine.build_meta("k", None, None);
        assert!(bare.refs.is_empty());
        assert!(bare.params.is_empty());
    }

    #[test]
    fn fallback_line_forms() {
        let engine = MessageTemplateEngine;
        let with_task = engine.build_meta(
            "s.handoff",
            Some(&[("task".to_string(), "t-3".to_string())]),
            None,
        );
        let line = engine.fallback_line(&with_task);
        assert!(line.contains("[s.handoff]"));
        assert!(line.contains("task_id=t-3"));
        assert!(line.contains("view_task"));
        let bare = engine.build_meta("s.notice", None, None);
        let line2 = engine.fallback_line(&bare);
        assert_eq!(line2, "[s.notice] — details unavailable.");
    }

    #[test]
    fn substitute_resolves_whitelisted_fields_and_missing() {
        let engine = MessageTemplateEngine;
        let t = task();
        let m = member();
        let params = BTreeMap::from([("feedback".to_string(), "good".to_string())]);
        let template = [
            "Task {{task.title}} by {{task.assignee}}",
            "reviewers: {{task.reviewer}}",
            "round {{task.review_round}}/{{task.max_review_rounds}}",
            "member {{member.display_name}} ({{member.desc}})",
            "note {{param.feedback}}",
            "unknown {{task.nope}} {{unknown.x}}",
        ]
        .join("\n");
        let out = engine.substitute(&template, Some(&t), Some(&m), &params);
        assert!(out.contains("Build the widget"), "{out}");
        assert!(out.contains("by alice"));
        assert!(out.contains("reviewers: bob, carol"));
        assert!(out.contains("round 2/3"));
        assert!(out.contains("member Alice (builder)"));
        assert!(out.contains("note good"));
        assert!(out.contains("<missing:task.nope>"), "{out}");
        assert!(out.contains("<missing:unknown.x>"), "{out}");
        // 空白容忍 {{ task.title }}。
        let spaced = engine.substitute("{{ task.title }}", Some(&t), None, &BTreeMap::new());
        assert_eq!(spaced, "Build the widget");
        // 值永不二次扫描:替换结果中的 {{...}} 保持原样。
        let inject = engine.substitute(
            "{{param.feedback}}",
            None,
            None,
            &BTreeMap::from([("feedback".to_string(), "{{task.title}}".to_string())]),
        );
        assert_eq!(inject, "{{task.title}}", "不重新扫描");
    }

    #[test]
    fn expand_ordinary_and_templated_and_fallback() {
        let engine = MessageTemplateEngine;
        // 普通消息透传。
        let plain = engine.expand("hello", None, "template", None, None);
        assert_eq!(plain.body, "hello");
        assert!(!plain.is_template);
        // 畸形 meta → 普通消息。
        let malformed = engine.expand("hi", Some("not json"), "t", None, None);
        assert_eq!(malformed.body, "hi");
        assert!(!malformed.is_template);
        // 模板消息全解析。
        let meta = r#"{"template":"s.handoff","refs":{"task":"t-1","member":"alice"}}"#;
        let rendered = engine.expand(
            "",
            Some(meta),
            "Task: {{task.title}} → {{member.display_name}}",
            Some(&task()),
            Some(&member()),
        );
        assert_eq!(rendered.body, "Task: Build the widget → Alice");
        assert!(rendered.is_template);
        // 引用行缺失 → fallback。
        let missing_task = engine.expand("", Some(meta), "t", None, Some(&member()));
        assert!(
            missing_task.body.contains("[s.handoff] task_id=t-1"),
            "{}",
            missing_task.body
        );
        assert!(missing_task.is_template);
        // 未知字段 → <missing:...>(不降级 fallback)。
        let unknown = engine.expand(
            "",
            Some(r#"{"template":"s.x","refs":{"task":"t-1"}}"#),
            "{{task.nope}}",
            Some(&task()),
            None,
        );
        assert_eq!(unknown.body, "<missing:task.nope>");
    }

    #[test]
    fn plugin_registers_message() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TeamMessagePlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let engine = ctx
            .service::<dyn TeamMessage>(&TEAM_MESSAGE)
            .expect("team-message seam");
        let out = engine.substitute("{{task.title}}", Some(&task()), None, &BTreeMap::new());
        assert_eq!(out, "Build the widget");
        drop(effects);
        assert!(!ctx.has_service(&TEAM_MESSAGE));
    }
}
