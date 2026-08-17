//! # ah-plugins-scheduler-render
//!
//! 真实调度消息组装(对齐 openjiuwen/agent_teams/agent/scheduling/render.py,F_62/F_63):
//! 纯函数,无 IO、无状态。
//! - meta_* 组装投递载荷 MessageMeta { template, refs, params }:
//!   refs = {"task": task_id}(投递时按*当前*任务行渲染,不快照任务正文);
//! - rework 的 params 携带*解析后*的轮数上限与已结算轮次的 fail 反馈聚合
//!   (任务行在投递时答不出这两个值);
//! - 空反馈显式兜底为 "无"(对齐 Python `feedback or t("scheduler.none")`);非空原样保留;
//! - format_fail_feedback 按输入顺序把 `- reviewer: feedback` 用 \n 连成归属块。

use std::collections::BTreeMap;
use std::sync::Arc;

use ah_contracts::keys::SCHEDULER_RENDER;
use ah_contracts::prelude::Effect;
use ah_contracts::scheduler_render::{
    _REVIEW_RENUDGE, _REVIEW_REQUEST, _REWORK, _TASK_START, _TASK_START_PLAN, _VERIFIED_REPORT,
    SchedulerRender, SchedulerRenderView,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_message::MessageMeta;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 空反馈的显式兜底文案(对齐 Python t("scheduler.none") = "无")。
const EMPTY_FEEDBACK_PLACEHOLDER: &str = "无";

/// 真实调度消息组装实现(纯函数)。
pub struct SchedulerRenderImpl;

impl Seam for SchedulerRenderImpl {}

impl SchedulerRender for SchedulerRenderImpl {
    fn meta_task_start(&self, view: &SchedulerRenderView) -> MessageMeta {
        let template = if view.planning {
            _TASK_START_PLAN
        } else {
            _TASK_START
        };
        Self::task_meta(template, &view.task_id)
    }

    fn meta_review_request(&self, view: &SchedulerRenderView) -> MessageMeta {
        Self::task_meta(_REVIEW_REQUEST, &view.task_id)
    }

    fn meta_review_renudge(&self, view: &SchedulerRenderView) -> MessageMeta {
        Self::task_meta(_REVIEW_RENUDGE, &view.task_id)
    }

    fn meta_rework(
        &self,
        view: &SchedulerRenderView,
        max_rounds: u64,
        feedback: &str,
    ) -> MessageMeta {
        let mut meta = Self::task_meta(_REWORK, &view.task_id);
        meta.params
            .insert("max_rounds".to_string(), max_rounds.to_string());
        let feedback = if feedback.is_empty() {
            // 显式兜底:空反馈 → "无"(对齐 Python `feedback or t("scheduler.none")`)。
            EMPTY_FEEDBACK_PLACEHOLDER.to_string()
        } else {
            feedback.to_string()
        };
        meta.params.insert("feedback".to_string(), feedback);
        meta
    }

    fn meta_verified_report(&self, view: &SchedulerRenderView) -> MessageMeta {
        Self::task_meta(_VERIFIED_REPORT, &view.task_id)
    }

    fn format_fail_feedback(&self, fail_feedback: &[(String, String)]) -> String {
        let lines: Vec<String> = fail_feedback
            .iter()
            .map(|(reviewer, feedback)| format!("- {reviewer}: {feedback}"))
            .collect();
        lines.join("\n")
    }
}

impl SchedulerRenderImpl {
    /// 组装带 refs={"task": task_id} 与空 params 的模板载荷。
    fn task_meta(template: &str, task_id: &str) -> MessageMeta {
        let mut refs = BTreeMap::new();
        refs.insert("task".to_string(), task_id.to_string());
        MessageMeta {
            template: template.to_string(),
            refs,
            params: BTreeMap::new(),
        }
    }
}

/// scheduler-render 插件:注册 `scheduler-render` seam。
pub struct SchedulerRenderPlugin;

impl Plugin for SchedulerRenderPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-scheduler-render"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SCHEDULER_RENDER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let render: Arc<dyn SchedulerRender> = Arc::new(SchedulerRenderImpl);
        Ok(vec![ctx.register(SCHEDULER_RENDER, render)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_hub::plugin::DynPlugin;

    fn view(task_id: &str, planning: bool) -> SchedulerRenderView {
        SchedulerRenderView {
            task_id: task_id.to_string(),
            title: "示例任务".to_string(),
            review_round: 1,
            status: if planning {
                "PLANNING".to_string()
            } else {
                "IN_PROGRESS".to_string()
            },
            planning,
        }
    }

    #[test]
    fn meta_task_start_picks_plan_template_when_planning() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_task_start(&view("t-1", true));
        assert_eq!(meta.template, _TASK_START_PLAN);
        assert_eq!(meta.refs.get("task").map(String::as_str), Some("t-1"));
        assert!(meta.params.is_empty());
    }

    #[test]
    fn meta_task_start_picks_normal_template_when_not_planning() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_task_start(&view("t-2", false));
        assert_eq!(meta.template, _TASK_START);
        assert_eq!(meta.refs.get("task").map(String::as_str), Some("t-2"));
        assert!(meta.params.is_empty());
    }

    #[test]
    fn meta_review_request_and_renudge_carry_task_ref() {
        let render = SchedulerRenderImpl;
        let v = view("t-3", false);
        let request = render.meta_review_request(&v);
        assert_eq!(request.template, _REVIEW_REQUEST);
        assert_eq!(request.refs.get("task").map(String::as_str), Some("t-3"));
        let renudge = render.meta_review_renudge(&v);
        assert_eq!(renudge.template, _REVIEW_RENUDGE);
        assert_eq!(renudge.refs.get("task").map(String::as_str), Some("t-3"));
        assert!(request.params.is_empty() && renudge.params.is_empty());
    }

    #[test]
    fn meta_rework_carries_resolved_rounds_and_feedback() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_rework(&view("t-4", false), 3, "结果不正确");
        assert_eq!(meta.template, _REWORK);
        assert_eq!(meta.refs.get("task").map(String::as_str), Some("t-4"));
        assert_eq!(meta.params.get("max_rounds").map(String::as_str), Some("3"));
        assert_eq!(
            meta.params.get("feedback").map(String::as_str),
            Some("结果不正确")
        );
    }

    #[test]
    fn meta_rework_falls_back_to_placeholder_on_empty_feedback() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_rework(&view("t-5", false), 0, "");
        assert_eq!(meta.params.get("max_rounds").map(String::as_str), Some("0"));
        assert_eq!(
            meta.params.get("feedback").map(String::as_str),
            Some(EMPTY_FEEDBACK_PLACEHOLDER),
            "空反馈显式兜底为 无 文案"
        );
    }

    #[test]
    fn meta_verified_report_uses_template_and_task_ref() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_verified_report(&view("t-6", false));
        assert_eq!(meta.template, _VERIFIED_REPORT);
        assert_eq!(meta.refs.get("task").map(String::as_str), Some("t-6"));
        assert!(meta.params.is_empty());
    }

    #[test]
    fn format_fail_feedback_joins_lines_in_input_order() {
        let render = SchedulerRenderImpl;
        let input = vec![
            ("alice".to_string(), "补测试".to_string()),
            ("bob".to_string(), "改命名".to_string()),
        ];
        assert_eq!(
            render.format_fail_feedback(&input),
            "- alice: 补测试\n- bob: 改命名"
        );
    }

    #[test]
    fn format_fail_feedback_empty_input_yields_empty_string() {
        let render = SchedulerRenderImpl;
        assert_eq!(render.format_fail_feedback(&[]), "");
    }

    #[test]
    fn message_meta_serde_roundtrip_preserves_payload() {
        let render = SchedulerRenderImpl;
        let meta = render.meta_rework(&view("t-7", true), 2, "重做");
        let json = serde_json::to_string(&meta).expect("serialize");
        let back: MessageMeta = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, meta);
        // JSON 形态:template 键 + refs/params 对象。
        let value: serde_json::Value = serde_json::from_str(&json).expect("json value");
        assert_eq!(value["template"].as_str(), Some(_REWORK));
        assert_eq!(value["refs"]["task"].as_str(), Some("t-7"));
        assert_eq!(value["params"]["max_rounds"].as_str(), Some("2"));
        assert_eq!(value["params"]["feedback"].as_str(), Some("重做"));
    }

    #[test]
    fn plugin_registers_scheduler_render_seam() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(SchedulerRenderPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let render = ctx
            .service::<dyn SchedulerRender>(&SCHEDULER_RENDER)
            .expect("scheduler-render seam");
        let meta = render.meta_task_start(&view("t-8", true));
        assert_eq!(meta.template, _TASK_START_PLAN);
        assert_eq!(render.format_fail_feedback(&[]), "");
        drop(effects);
        assert!(!ctx.has_service(&SCHEDULER_RENDER));
    }
}
