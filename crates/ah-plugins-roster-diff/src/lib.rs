//! # ah-plugins-roster-diff
//!
//! Real team roster diff + message-body rendering (aligned with
//! openjiuwen/agent_teams/prompts/messages.py):
//! - diff_roster: keyed by member_name, tracks display_name/desc/role only;
//! - format_member_line: one roster row with optional [human] tag and [prefix];
//! - snapshot (first time) and delta (changes) bilingual bodies.

use std::collections::HashMap;
use std::sync::Arc;

use ah_contracts::keys::ROSTER_DIFF;
use ah_contracts::prelude::Effect;
use ah_contracts::roster_diff::{RosterDelta, RosterDiff, RosterMember};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 跟踪字段:display_name/desc/role(member_name 是键,运行时状态不跟踪)。
const TRACKED_FIELDS: [&str; 3] = ["display_name", "desc", "role"];

/// 双语标签(消息正文;对齐 _LABELS)。
fn labels(language: &str) -> [&'static str; 5] {
    match language {
        "en" => [
            "# Relationships",
            "# Roster Change",
            "joined",
            "left",
            "updated",
        ],
        _ => ["# 成员关系", "# 成员变更", "加入", "退出", "信息更新"],
    }
}

fn field<'a>(member: &'a RosterMember, field: &str) -> &'a str {
    match field {
        "display_name" => &member.display_name,
        "desc" => &member.desc,
        "role" => &member.role,
        _ => "",
    }
}

/// 真实名册 diff 实现。
pub struct RosterDiffEngine;

impl Seam for RosterDiffEngine {}

impl RosterDiff for RosterDiffEngine {
    fn diff(&self, old: Option<&[RosterMember]>, new: Option<&[RosterMember]>) -> RosterDelta {
        let old_by_name: HashMap<&str, &RosterMember> = old
            .unwrap_or(&[])
            .iter()
            .map(|m| (m.member_name.as_str(), m))
            .collect();
        let new_by_name: HashMap<&str, &RosterMember> = new
            .unwrap_or(&[])
            .iter()
            .map(|m| (m.member_name.as_str(), m))
            .collect();
        let mut joined: Vec<RosterMember> = Vec::new();
        let mut left: Vec<RosterMember> = Vec::new();
        let mut changed: Vec<RosterMember> = Vec::new();
        for (name, member) in &new_by_name {
            if !old_by_name.contains_key(name) {
                joined.push((*member).clone());
            }
        }
        for (name, member) in &old_by_name {
            if !new_by_name.contains_key(name) {
                left.push((*member).clone());
            }
        }
        for (name, member) in &new_by_name {
            let Some(previous) = old_by_name.get(name) else {
                continue;
            };
            let track_changed = TRACKED_FIELDS
                .iter()
                .any(|f| field(member, f) != field(previous, f));
            if track_changed {
                changed.push((*member).clone());
            }
        }
        RosterDelta {
            joined,
            left,
            changed,
        }
    }

    fn format_member_line(
        &self,
        member: &RosterMember,
        mark_humans: bool,
        prefix: Option<&str>,
    ) -> String {
        let head = match prefix {
            Some(p) => format!("- [{p}] "),
            None => "- ".to_string(),
        };
        let mut line = format!(
            "{head}member_name={} display_name={}",
            member.member_name,
            if member.display_name.is_empty() {
                "unknown"
            } else {
                &member.display_name
            },
        );
        if mark_humans && member.role == "human_agent" {
            line.push_str(" [human]");
        }
        if !member.desc.is_empty() {
            line.push_str(&format!(" :: {}", member.desc));
        }
        line
    }

    fn build_roster_snapshot_text(
        &self,
        members: Option<&[RosterMember]>,
        mark_humans: bool,
        language: &str,
    ) -> Option<String> {
        let members = members?;
        if members.is_empty() {
            return None;
        }
        let [heading, ..] = labels(language);
        let rows: Vec<String> = members
            .iter()
            .map(|m| self.format_member_line(m, mark_humans, None))
            .collect();
        Some(format!("{heading}\n\n{}\n", rows.join("\n")))
    }

    fn build_roster_delta_text(
        &self,
        delta: &RosterDelta,
        mark_humans: bool,
        language: &str,
    ) -> Option<String> {
        if delta.is_empty() {
            return None;
        }
        let [_, heading, joined_l, left_l, updated_l] = labels(language);
        let mut rows: Vec<String> = Vec::new();
        for member in &delta.joined {
            rows.push(self.format_member_line(member, mark_humans, Some(joined_l)));
        }
        for member in &delta.left {
            rows.push(self.format_member_line(member, mark_humans, Some(left_l)));
        }
        for member in &delta.changed {
            rows.push(self.format_member_line(member, mark_humans, Some(updated_l)));
        }
        Some(format!("{heading}\n\n{}\n", rows.join("\n")))
    }
}

/// roster-diff 插件:注册 `roster-diff` seam。
pub struct RosterDiffPlugin;

impl Plugin for RosterDiffPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-roster-diff"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ROSTER_DIFF]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let engine: Arc<dyn RosterDiff> = Arc::new(RosterDiffEngine);
        Ok(vec![ctx.register(ROSTER_DIFF, engine)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::ROSTER_DIFF;
    use ah_hub::plugin::DynPlugin;

    fn member(name: &str, display: &str, desc: &str, role: &str) -> RosterMember {
        RosterMember {
            member_name: name.to_string(),
            display_name: display.to_string(),
            desc: desc.to_string(),
            role: role.to_string(),
        }
    }

    #[test]
    fn diff_detects_joined_left_and_changed() {
        let engine = RosterDiffEngine;
        let old = vec![
            member("alice", "Alice", "builder", "teammate"),
            member("bob", "Bob", "reviewer", "teammate"),
        ];
        let new = vec![
            member("alice", "Alice", "lead builder", "teammate"), // desc 变更
            member("carol", "Carol", "", "teammate"),             // 新加入
        ];
        let delta = engine.diff(Some(&old), Some(&new));
        assert_eq!(delta.joined.len(), 1);
        assert_eq!(delta.joined[0].member_name, "carol");
        assert_eq!(delta.left.len(), 1);
        assert_eq!(delta.left[0].member_name, "bob");
        assert_eq!(delta.changed.len(), 1);
        assert_eq!(delta.changed[0].member_name, "alice");
        assert!(!delta.is_empty());
    }

    #[test]
    fn diff_ignores_untracked_fields_and_empty_rosters() {
        let engine = RosterDiffEngine;
        // None 视为空。
        let delta = engine.diff(None, None);
        assert!(delta.is_empty());
        let from_empty = engine.diff(None, Some(&[member("a", "A", "", "")]));
        assert_eq!(from_empty.joined.len(), 1);
        // 同字段不变 → 无 changed。
        let same = engine.diff(
            Some(&[member("a", "A", "d", "r")]),
            Some(&[member("a", "A", "d", "r")]),
        );
        assert!(same.is_empty());
        // 无变化也 empty。
        let delta2 = engine.diff(Some(&[]), Some(&[]));
        assert!(delta2.is_empty());
    }

    #[test]
    fn format_member_line_variants() {
        let engine = RosterDiffEngine;
        let m = member("alice", "Alice", "builder", "teammate");
        assert_eq!(
            engine.format_member_line(&m, false, None),
            "- member_name=alice display_name=Alice :: builder"
        );
        assert_eq!(
            engine.format_member_line(&m, false, Some("加入")),
            "- [加入] member_name=alice display_name=Alice :: builder"
        );
        // 无 display_name → unknown。
        let anon = member("bob", "", "", "teammate");
        assert_eq!(
            engine.format_member_line(&anon, false, None),
            "- member_name=bob display_name=unknown"
        );
        // human_agent + mark_humans → [human]。
        let human = member("hum", "Hum", "", "human_agent");
        assert_eq!(
            engine.format_member_line(&human, true, None),
            "- member_name=hum display_name=Hum [human]"
        );
        // mark_humans=false 不标记 human。
        assert_eq!(
            engine.format_member_line(&human, false, None),
            "- member_name=hum display_name=Hum"
        );
    }

    #[test]
    fn snapshot_and_delta_bodies_bilingual() {
        let engine = RosterDiffEngine;
        let members = vec![member("alice", "Alice", "", "teammate")];
        let cn = engine
            .build_roster_snapshot_text(Some(&members), false, "cn")
            .expect("cn");
        assert!(cn.starts_with("# 成员关系"));
        assert!(cn.contains("- member_name=alice"));
        let en = engine
            .build_roster_snapshot_text(Some(&members), false, "en")
            .expect("en");
        assert!(en.starts_with("# Relationships"));
        // 空名册 → None。
        assert!(
            engine
                .build_roster_snapshot_text(Some(&[]), false, "cn")
                .is_none()
        );

        let delta = RosterDelta {
            joined: vec![member("carol", "Carol", "", "")],
            left: vec![member("bob", "Bob", "", "")],
            changed: vec![member("alice", "Alice", "new", "")],
        };
        let dcn = engine
            .build_roster_delta_text(&delta, false, "cn")
            .expect("delta");
        assert!(dcn.starts_with("# 成员变更"));
        assert!(dcn.contains("- [加入] member_name=carol"));
        assert!(dcn.contains("- [退出] member_name=bob"));
        assert!(dcn.contains("- [信息更新] member_name=alice"));
        let den = engine
            .build_roster_delta_text(&delta, false, "en")
            .expect("delta en");
        assert!(den.contains("- [joined] member_name=carol"));
        // 空差异 → None。
        assert!(
            engine
                .build_roster_delta_text(
                    &RosterDelta {
                        joined: vec![],
                        left: vec![],
                        changed: vec![]
                    },
                    false,
                    "cn"
                )
                .is_none()
        );
    }

    #[test]
    fn plugin_registers_roster_diff() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(RosterDiffPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let engine = ctx
            .service::<dyn RosterDiff>(&ROSTER_DIFF)
            .expect("roster-diff seam");
        let delta = engine.diff(None, Some(&[member("a", "A", "", "")]));
        assert_eq!(delta.joined.len(), 1);
        drop(effects);
        assert!(!ctx.has_service(&ROSTER_DIFF));
    }
}
