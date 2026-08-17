//! scheduler-render seam:调度消息组装(对齐 openjiuwen/agent_teams/agent/scheduling/render.py,F_62/F_63)。
//!
//! 两类收件人两套机制:
//! - **成员**交接只组投递载荷(meta = {template, refs, params}),消息行 content 为空;文案在
//!   prompts/<lang>/scheduler_*.md,投递时按收件人语言对*当前*任务行渲染(F_63)——能表查的
//!   一律进 refs 现查,params 只放表答不出的瞬时值(某轮 fail feedback 聚合、解析后的轮数上限);
//! - **leader** 摘要/升级走 deliver_input 直投,是 i18n.t 一行短串,不经本 seam(调度器运行时
//!   直接调用 crate::team_i18n::TeamI18n)。
//!
//! 契约零实现:MessageMeta 组装与反馈聚合由插件提供(如 ah-plugins-scheduler-render)。

use crate::seam::Seam;
use crate::team_message::MessageMeta;

/// 模板键:开工指令(非 plan 态),prompts/<lang>/scheduler_task_start.md 的 basename。
pub const _TASK_START: &str = "scheduler_task_start";

/// 模板键:开工指令(plan 态,plan gate 感知)。
pub const _TASK_START_PLAN: &str = "scheduler_task_start_plan";

/// 模板键:送审(新开一轮,发给该轮一位 reviewer)。
pub const _REVIEW_REQUEST: &str = "scheduler_review_request";

/// 模板键:催办(开轮后未投票的 reviewer)。
pub const _REVIEW_RENUDGE: &str = "scheduler_review_renudge";

/// 模板键:返工(一轮失败结算后发给作者)。
pub const _REWORK: &str = "scheduler_rework";

/// 模板键:验证通过后请作者向 leader 汇报结果。
pub const _VERIFIED_REPORT: &str = "scheduler_verified_report";

/// 本 seam 的服务键(定义于 crate::keys,此处再导出供契约路径使用)。
pub use crate::keys::SCHEDULER_RENDER;

/// meta 构建所需的任务字段视图(白名单投影;planning 由调用方按任务 status 判定)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SchedulerRenderView {
    /// 任务 id(进 refs["task"],投递时按当前任务行渲染)。
    pub task_id: String,
    /// 任务标题(元数据/诊断用,不进 payload)。
    pub title: String,
    /// 当前评审轮数。
    pub review_round: u64,
    /// 任务状态(如 PLANNING / IN_PROGRESS / IN_REVIEW)。
    pub status: String,
    /// 任务是否处于 plan 态(决定开工模板选 _TASK_START_PLAN 还是 _TASK_START)。
    pub planning: bool,
}

/// 调度消息组装 Seam(Service Definition):纯函数,无 IO、无状态。
pub trait SchedulerRender: Seam {
    /// 开工指令投递载荷:plan 态选 _TASK_START_PLAN,否则 _TASK_START;refs={task: task_id}。
    fn meta_task_start(&self, view: &SchedulerRenderView) -> MessageMeta;

    /// 送审投递载荷:_REVIEW_REQUEST;refs={task: task_id}。
    fn meta_review_request(&self, view: &SchedulerRenderView) -> MessageMeta;

    /// 催办投递载荷:_REVIEW_RENUDGE;refs={task: task_id}。
    fn meta_review_renudge(&self, view: &SchedulerRenderView) -> MessageMeta;

    /// 返工投递载荷:_REWORK;refs={task: task_id},
    /// params={max_rounds: 解析后的轮数上限, feedback: 已结算轮次的 fail 反馈聚合}。
    fn meta_rework(
        &self,
        view: &SchedulerRenderView,
        max_rounds: u64,
        feedback: &str,
    ) -> MessageMeta;

    /// 验证通过后的汇报请求投递载荷:_VERIFIED_REPORT;refs={task: task_id}。
    fn meta_verified_report(&self, view: &SchedulerRenderView) -> MessageMeta;

    /// 把按评审者聚合的 fail 反馈连成归属块:`- reviewer: feedback` 逐行 `\n` 连接;
    /// 空输入 → 空串。
    fn format_fail_feedback(&self, fail_feedback: &[(String, String)]) -> String;
}
