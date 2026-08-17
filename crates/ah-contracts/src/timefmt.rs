//! timefmt seam:毫秒 epoch → 人类可读时间渲染(对齐 openjiuwen/agent_teams/timefmt.py)。
//!
//! 团队数据库存毫秒 UTC epoch(易排序/比较/索引),但裸 epoch 对 LLM 无法可靠推理
//! 顺序与"多久以前",破坏消息延迟到达时的优先级判断。本 seam 把它渲染成
//! `<绝对本地时间> (<相对差>)`,如 `2026-05-27 14:30:05 +08:00 (3 分钟前)`。
//!
//! 相对桶选择是纯数值运算(语言无关),文案键为字符串常量(渲染由插件按语言完成);
//! 绝对时间渲染为手写 epoch→日历换算 + 时区偏移标注,保证跨机器可对齐。
//!
//! 契约零实现:epoch→日历换算 / 系统本地时区解析由插件提供(如 ah-plugins-timefmt)。

use crate::seam::Seam;

/// 相对时间 i18n 键:刚刚(无数值占位)。
pub const KEY_JUST_NOW: &str = "time.just_now";
/// 相对时间 i18n 键:N 秒前。
pub const KEY_SECONDS_AGO: &str = "time.seconds_ago";
/// 相对时间 i18n 键:N 分钟前。
pub const KEY_MINUTES_AGO: &str = "time.minutes_ago";
/// 相对时间 i18n 键:N 小时前。
pub const KEY_HOURS_AGO: &str = "time.hours_ago";
/// 相对时间 i18n 键:N 天前。
pub const KEY_DAYS_AGO: &str = "time.days_ago";
/// 相对时间 i18n 键:时间未知(timestamp 为 None)。
pub const KEY_UNKNOWN: &str = "time.unknown";

/// 相对时间桶:选中的 i18n 键 + 可选数值。
///
/// `value` 为 `None` 表示 just_now 桶(键不带数值占位)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelativeBucket {
    /// i18n 键(见 [`KEY_JUST_NOW`] 等常量)。
    pub key: &'static str,
    /// 桶数值;just_now 桶为 `None`。
    pub value: Option<u64>,
}

impl RelativeBucket {
    /// 构造一个相对桶。
    pub const fn new(key: &'static str, value: Option<u64>) -> Self {
        Self { key, value }
    }
}

/// timefmt Seam(Service Definition):纯函数,无 IO、无状态。
///
/// 实现方负责真实 epoch→日历换算与本地时区解析(对齐 Python
/// `datetime.fromtimestamp(...).astimezone()` 的本地时区语义)。
pub trait Timefmt: Seam {
    /// 相对时间桶选择:`delta_ms = now_ms - timestamp_ms`,正数表示过去。
    ///
    /// 负数(时钟漂移,时间戳在未来)与 <10s 都归 `just_now`,绝不渲染负数计数。
    fn relative_key_and_value(&self, delta_ms: i64) -> RelativeBucket;

    /// 绝对时间渲染(系统本地时区):`YYYY-MM-DD HH:MM:SS ±HH:MM`。
    fn format_absolute(&self, timestamp_ms: i64) -> String;

    /// 绝对时间渲染(UTC):`YYYY-MM-DD HH:MM:SS +00:00`。
    fn format_absolute_utc(&self, timestamp_ms: i64) -> String;

    /// 绝对时间渲染(固定时区偏移秒,东为正):`YYYY-MM-DD HH:MM:SS ±HH:MM`。
    ///
    /// 纯函数、确定性,用于注入固定偏移的可测路径。
    fn format_absolute_with_offset(&self, timestamp_ms: i64, offset_secs: i32) -> String;

    /// 时间上下文渲染:`<absolute> (<relative>)`;`None` → 未知文案
    /// (`time.unknown` 键,如 `时间未知`)。
    fn format_time_context(&self, timestamp_ms: Option<i64>, now_ms: i64) -> String;
}
