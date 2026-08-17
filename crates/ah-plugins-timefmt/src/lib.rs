//! # ah-plugins-timefmt
//!
//! 真实毫秒 epoch → 人类可读时间渲染(1:1 对齐 openjiuwen/agent_teams/timefmt.py):
//! - 相对桶:`delta_ms < 0`(未来时钟偏差)或 < 10_000ms → just_now;
//!   < 60_000ms → seconds_ago;< 3_600_000ms → minutes_ago;
//!   < 86_400_000ms → hours_ago;否则 days_ago;
//! - 绝对时间:手写 epoch → 本地日历(Howard Hinnant civil 算法,含负数 epoch),
//!   附 `±HH:MM` 时区偏移;构造时可注入固定偏移(确定性、可测);
//! - 系统本地时区偏移解析顺序:`date +%z` 系统命令(真实系统调用,正确处理命名时区,
//!   如 TZ=Asia/Shanghai)→ `TZ` 环境变量简单 POSIX 偏移解析 → 两者均不可用则
//!   **显式降级**为 UTC(0) 并打印一次说明(不是静默 fallback);
//! - 时间上下文:`<absolute> (<relative>)`,中文文案直渲染(对齐 i18n.py cn 表,
//!   无 i18n 模块,键为字符串常量)。

use std::process::Command;
use std::sync::{Arc, OnceLock};

use ah_contracts::keys::TIMEFMT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::timefmt::{
    KEY_DAYS_AGO, KEY_HOURS_AGO, KEY_JUST_NOW, KEY_MINUTES_AGO, KEY_SECONDS_AGO, KEY_UNKNOWN,
    RelativeBucket, Timefmt,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 相对桶边界(秒,对齐 Python 常量)。
const JUST_NOW_SECONDS: i64 = 10;
const MINUTE_SECONDS: i64 = 60;
const HOUR_SECONDS: i64 = 60 * 60;
const DAY_SECONDS: i64 = 24 * 60 * 60;

// ---------------------------------------------------------------------------
// 纯函数核心(可测,无 IO)
// ---------------------------------------------------------------------------

/// 自 1970-01-01 起的天数 → (年, 月, 日)(Howard Hinnant civil_from_days)。
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 毫秒 epoch → `YYYY-MM-DD HH:MM:SS ±HH:MM`(固定偏移秒,东为正)。
fn render_absolute(timestamp_ms: i64, offset_secs: i32) -> String {
    let epoch_secs = timestamp_ms.div_euclid(1000);
    let local_secs = epoch_secs + i64::from(offset_secs);
    let days = local_secs.div_euclid(86_400);
    let secs_of_day = local_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    format!(
        "{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} {sign}{:02}:{:02}",
        abs / 3600,
        (abs % 3600) / 60
    )
}

/// 解析 `±HH[:MM]`(ISO 方向:东为正)为偏移秒。
fn parse_iso_offset(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let (sign, rest) = match bytes[0] {
        b'+' => (1i32, &bytes[1..]),
        b'-' => (-1i32, &bytes[1..]),
        _ => return None,
    };
    // 小时至少 1 位(POSIX TZ 允许 `+8` 单数字小时)。
    let hh = match rest.len() {
        1 => digits(&rest[..1])?,
        _ => digits(&rest[..2])?,
    };
    let mm = match rest.len() {
        1 | 2 => 0,
        4 => digits(&rest[2..])?,
        5 if rest[2] == b':' => digits(&rest[3..])?,
        _ => return None,
    };
    if hh > 23 || mm > 59 {
        return None;
    }
    Some(sign * (hh * 3600 + mm * 60))
}

fn digits(bytes: &[u8]) -> Option<i32> {
    if bytes.iter().any(|b| !b.is_ascii_digit()) {
        return None;
    }
    bytes.iter().try_fold(0i32, |acc, b| {
        acc.checked_mul(10)?.checked_add(i32::from(b - b'0'))
    })
}

/// POSIX TZ 偏移解析(`std offset` 或纯 `offset`):返回本地偏移秒(东为正)。
///
/// POSIX 符号约定与 ISO 相反:`offset` 是加到本地时间得 UTC 的加数,正 = 西、
/// 负 = 东。如 `TZ=UTC+8` → 西八区 → -28800;`TZ=CST-8` → 东八区 → +28800;
/// `TZ=+08:00`(纯偏移)同样表示西八区。与 `date +%z` 的输出方向一致。
/// dst 后缀(如 `EST5EDT` 的 `EDT`)忽略,只取 std 偏移;分钟级偏移按 POSIX
/// hh:mm 合法解析(个别系统 `date` 会拒绝分钟级纯偏移,本路径仅作无 `date`
/// 时的兜底)。
fn posix_offset_secs(tz: &str) -> Option<i32> {
    // 纯偏移形式(POSIX 符号:正 = 西,负 = 东;ISO 解析后再取反)。
    if let Some(rest) = tz.strip_prefix('+') {
        return Some(-parse_iso_offset(&format!("+{rest}"))?);
    }
    if let Some(rest) = tz.strip_prefix('-') {
        return Some(-parse_iso_offset(&format!("-{rest}"))?);
    }
    // `std offset` 形式:std 为字母序列,offset 从首个数字/符号开始。
    let Some(digits_start) = tz.find(|c: char| c.is_ascii_digit() || c == '+' || c == '-') else {
        // 纯命名时区无偏移:仅 UTC/GMT 系可无时区数据库判定。
        return if tz == "UTC" || tz == "GMT" || tz == "Etc/UTC" || tz == "Etc/GMT" {
            Some(0)
        } else {
            None
        };
    };
    let (_std, offset_part) = tz.split_at(digits_start);
    let (sign, num) = if let Some(rest) = offset_part.strip_prefix('+') {
        (1i32, rest)
    } else if let Some(rest) = offset_part.strip_prefix('-') {
        (-1i32, rest)
    } else {
        (1i32, offset_part)
    };
    // 截到数字/冒号前缀(dst 后缀如 `EST5EDT` 的 `EDT` 忽略)。
    let num_end = num
        .find(|c: char| !c.is_ascii_digit() && c != ':')
        .unwrap_or(num.len());
    if num_end == 0 {
        return None;
    }
    // 显式 '+' 或隐式(无符号)→ 西 → 本地偏移为负;'-' → 东 → 正。
    let v = parse_iso_offset(&format!("+{}", &num[..num_end]))?;
    Some(if sign == 1 { -v } else { v })
}

/// `date +%z` → 本地偏移秒(东为正)。
fn offset_from_date_command() -> Option<i32> {
    let output = Command::new("date").arg("+%z").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    parse_iso_offset(text.trim())
}

/// 从 `TZ` 环境变量解析本地偏移秒(东为正)。
fn offset_from_tz_env() -> Option<i32> {
    let tz = std::env::var("TZ").ok()?;
    if tz.trim().is_empty() || tz.trim_start().starts_with(':') {
        return None; // ':' 前缀是 tzfile 路径,需要时区数据库。
    }
    posix_offset_secs(tz.trim())
}

/// 系统本地时区偏移(秒,东为正),进程内解析一次并缓存。
///
/// 解析顺序:
/// 1. 系统命令 `date +%z`(真实系统调用;正确处理命名时区,如 TZ=Asia/Shanghai);
/// 2. `TZ` 环境变量简单 POSIX 偏移解析(仅当 `date` 不可用时);
/// 3. 两者均不可用 → **显式降级**为 UTC(0) 并打印一次说明(文档已注明)。
fn system_local_offset_secs() -> i32 {
    static LOCAL_OFFSET: OnceLock<i32> = OnceLock::new();
    *LOCAL_OFFSET.get_or_init(|| {
        if let Some(secs) = offset_from_date_command() {
            return secs;
        }
        if let Some(secs) = offset_from_tz_env() {
            return secs;
        }
        eprintln!(
            "timefmt: cannot resolve system local tz (no `date`, unparseable TZ); \
             degrading explicitly to UTC (+00:00)"
        );
        0
    })
}

/// 中文相对文案(对齐 i18n.py cn 表)。
fn relative_text(key: &str, value: Option<u64>) -> String {
    match (key, value) {
        (KEY_JUST_NOW, _) => "刚刚".to_string(),
        (KEY_SECONDS_AGO, Some(v)) => format!("{v} 秒前"),
        (KEY_MINUTES_AGO, Some(v)) => format!("{v} 分钟前"),
        (KEY_HOURS_AGO, Some(v)) => format!("{v} 小时前"),
        (KEY_DAYS_AGO, Some(v)) => format!("{v} 天前"),
        (KEY_UNKNOWN, _) => "时间未知".to_string(),
        // 桶键只由 relative_key_and_value 产出;出现其它键是插件内部契约违约,显式报错。
        (other, _) => unreachable!("timefmt: 未知相对键 `{other}`"),
    }
}

// ---------------------------------------------------------------------------
// Seam 实现
// ---------------------------------------------------------------------------

/// 真实 timefmt 实现(纯函数,无 IO、无状态)。
pub struct TimefmtService {
    /// 固定时区偏移秒(东为正);`None` → 运行时解析系统本地偏移。
    offset_secs: Option<i32>,
}

impl TimefmtService {
    /// 默认实例:绝对时间使用系统本地时区(见 `system_local_offset_secs`)。
    pub fn new() -> Self {
        Self { offset_secs: None }
    }

    /// 注入固定时区偏移(秒,东为正):确定性、可测。
    pub fn with_offset(offset_secs: i32) -> Self {
        Self {
            offset_secs: Some(offset_secs),
        }
    }

    /// 本实例使用的时区偏移秒。
    fn offset(&self) -> i32 {
        self.offset_secs.unwrap_or_else(system_local_offset_secs)
    }
}

impl Default for TimefmtService {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for TimefmtService {}

impl Timefmt for TimefmtService {
    fn relative_key_and_value(&self, delta_ms: i64) -> RelativeBucket {
        // 负数 delta(时间戳在未来,时钟漂移)与 <10s 均归 just_now,绝不渲染负数。
        if delta_ms < 0 {
            return RelativeBucket::new(KEY_JUST_NOW, None);
        }
        let seconds = delta_ms / 1000;
        if seconds < JUST_NOW_SECONDS {
            return RelativeBucket::new(KEY_JUST_NOW, None);
        }
        if seconds < MINUTE_SECONDS {
            return RelativeBucket::new(KEY_SECONDS_AGO, Some(seconds as u64));
        }
        if seconds < HOUR_SECONDS {
            return RelativeBucket::new(KEY_MINUTES_AGO, Some((seconds / MINUTE_SECONDS) as u64));
        }
        if seconds < DAY_SECONDS {
            return RelativeBucket::new(KEY_HOURS_AGO, Some((seconds / HOUR_SECONDS) as u64));
        }
        RelativeBucket::new(KEY_DAYS_AGO, Some((seconds / DAY_SECONDS) as u64))
    }

    fn format_absolute(&self, timestamp_ms: i64) -> String {
        render_absolute(timestamp_ms, self.offset())
    }

    fn format_absolute_utc(&self, timestamp_ms: i64) -> String {
        render_absolute(timestamp_ms, 0)
    }

    fn format_absolute_with_offset(&self, timestamp_ms: i64, offset_secs: i32) -> String {
        render_absolute(timestamp_ms, offset_secs)
    }

    fn format_time_context(&self, timestamp_ms: Option<i64>, now_ms: i64) -> String {
        let Some(ts) = timestamp_ms else {
            return relative_text(KEY_UNKNOWN, None);
        };
        let bucket = self.relative_key_and_value(now_ms - ts);
        let relative = relative_text(bucket.key, bucket.value);
        format!("{} ({relative})", self.format_absolute(ts))
    }
}

/// timefmt 插件:注册 `timefmt` seam。
pub struct TimefmtPlugin {
    offset_secs: Option<i32>,
}

impl TimefmtPlugin {
    /// 默认插件:绝对时间使用系统本地时区。
    pub fn new() -> Self {
        Self { offset_secs: None }
    }

    /// 注入固定时区偏移的插件(确定性部署)。
    pub fn with_offset(offset_secs: i32) -> Self {
        Self {
            offset_secs: Some(offset_secs),
        }
    }
}

impl Default for TimefmtPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugin for TimefmtPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-timefmt"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TIMEFMT]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let service: Arc<dyn Timefmt> = match self.offset_secs {
            Some(secs) => Arc::new(TimefmtService::with_offset(secs)),
            None => Arc::new(TimefmtService::new()),
        };
        Ok(vec![ctx.register(TIMEFMT, service)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::timefmt::Timefmt;
    use ah_hub::plugin::DynPlugin;

    /// 固定点:2026-05-27 06:30:05 UTC(epoch 毫秒)。
    const TS: i64 = 1_779_863_405_000;

    #[test]
    fn relative_bucket_boundaries() {
        let svc = TimefmtService::new();
        // 0ms / 9999ms → just_now(<10s)。
        assert_eq!(
            svc.relative_key_and_value(0),
            RelativeBucket::new(KEY_JUST_NOW, None)
        );
        assert_eq!(
            svc.relative_key_and_value(9_999),
            RelativeBucket::new(KEY_JUST_NOW, None)
        );
        // 10s → seconds_ago。
        assert_eq!(
            svc.relative_key_and_value(10_000),
            RelativeBucket::new(KEY_SECONDS_AGO, Some(10))
        );
        // 60s → minutes_ago。
        assert_eq!(
            svc.relative_key_and_value(60_000),
            RelativeBucket::new(KEY_MINUTES_AGO, Some(1))
        );
        // 3600s → hours_ago。
        assert_eq!(
            svc.relative_key_and_value(3_600_000),
            RelativeBucket::new(KEY_HOURS_AGO, Some(1))
        );
        // 86400s → days_ago。
        assert_eq!(
            svc.relative_key_and_value(86_400_000),
            RelativeBucket::new(KEY_DAYS_AGO, Some(1))
        );
        // 负数(未来时钟偏差)→ just_now,绝不渲染负数。
        assert_eq!(
            svc.relative_key_and_value(-1),
            RelativeBucket::new(KEY_JUST_NOW, None)
        );
        assert_eq!(
            svc.relative_key_and_value(-86_400_000),
            RelativeBucket::new(KEY_JUST_NOW, None)
        );
    }

    #[test]
    fn relative_bucket_values() {
        let svc = TimefmtService::new();
        // seconds = delta_ms // 1000。
        assert_eq!(
            svc.relative_key_and_value(42_000),
            RelativeBucket::new(KEY_SECONDS_AGO, Some(42))
        );
        // minutes = seconds // 60。
        assert_eq!(
            svc.relative_key_and_value(90_000),
            RelativeBucket::new(KEY_MINUTES_AGO, Some(1))
        );
        // 120s = 2 分钟(60s ≤ delta < 3600s 属于 minutes_ago)。
        assert_eq!(
            svc.relative_key_and_value(120_000),
            RelativeBucket::new(KEY_MINUTES_AGO, Some(2))
        );
        // 5400s = 1.5h → 已跨入 hours_ago 桶。
        assert_eq!(
            svc.relative_key_and_value(5_400_000),
            RelativeBucket::new(KEY_HOURS_AGO, Some(1))
        );
        // hours = seconds // 3600。
        assert_eq!(
            svc.relative_key_and_value(7_200_000),
            RelativeBucket::new(KEY_HOURS_AGO, Some(2))
        );
        // days = seconds // 86400。
        assert_eq!(
            svc.relative_key_and_value(259_200_000),
            RelativeBucket::new(KEY_DAYS_AGO, Some(3))
        );
    }

    #[test]
    fn absolute_utc_fixed_point() {
        let svc = TimefmtService::new();
        assert_eq!(svc.format_absolute_utc(TS), "2026-05-27 06:30:05 +00:00");
        assert_eq!(
            svc.format_absolute_with_offset(TS, 0),
            "2026-05-27 06:30:05 +00:00"
        );
    }

    #[test]
    fn absolute_with_injected_offset() {
        // +08:00(东八区)。
        let east = TimefmtService::with_offset(28_800);
        assert_eq!(east.format_absolute(TS), "2026-05-27 14:30:05 +08:00");
        assert_eq!(
            east.format_absolute_with_offset(TS, 28_800),
            "2026-05-27 14:30:05 +08:00"
        );
        // -05:00(西五区):06:30:05 UTC - 5h = 01:30:05。
        let west = TimefmtService::with_offset(-18_000);
        assert_eq!(west.format_absolute(TS), "2026-05-27 01:30:05 -05:00");
        // 半小时偏移(如印度 +05:30)。
        let half = TimefmtService::with_offset(19_800);
        assert_eq!(half.format_absolute(TS), "2026-05-27 12:00:05 +05:30");
    }

    #[test]
    fn absolute_epoch_edges_and_negative() {
        let svc = TimefmtService::new();
        // epoch 0。
        assert_eq!(svc.format_absolute_utc(0), "1970-01-01 00:00:00 +00:00");
        // -1s(1970 前):div_euclid 向下取整,与 Python floor 一致。
        assert_eq!(
            svc.format_absolute_utc(-1_000),
            "1969-12-31 23:59:59 +00:00"
        );
        // -1800s。
        assert_eq!(
            svc.format_absolute_utc(-1_800_000),
            "1969-12-31 23:30:00 +00:00"
        );
        // 闰年 2 月 29 日(2024-02-29 12:00:00 UTC)。
        assert_eq!(
            svc.format_absolute_utc(1_709_208_000_000),
            "2024-02-29 12:00:00 +00:00"
        );
    }

    #[test]
    fn time_context_renders_absolute_and_relative() {
        // +08:00 注入:2026-05-27 14:30:05,now = ts + 3 分钟。
        let svc = TimefmtService::with_offset(28_800);
        assert_eq!(
            svc.format_time_context(Some(TS), TS + 180_000),
            "2026-05-27 14:30:05 +08:00 (3 分钟前)"
        );
        // 各相对桶中文文案。
        assert_eq!(
            svc.format_time_context(Some(TS), TS + 59_000),
            "2026-05-27 14:30:05 +08:00 (59 秒前)"
        );
        assert_eq!(
            svc.format_time_context(Some(TS), TS + 3_600_000),
            "2026-05-27 14:30:05 +08:00 (1 小时前)"
        );
        assert_eq!(
            svc.format_time_context(Some(TS), TS + 86_400_000),
            "2026-05-27 14:30:05 +08:00 (1 天前)"
        );
        // 0ms 差与未来偏差都归"刚刚"。
        assert_eq!(
            svc.format_time_context(Some(TS), TS),
            "2026-05-27 14:30:05 +08:00 (刚刚)"
        );
        assert_eq!(
            svc.format_time_context(Some(TS), TS - 5_000),
            "2026-05-27 14:30:05 +08:00 (刚刚)"
        );
    }

    #[test]
    fn time_context_none_is_unknown() {
        let svc = TimefmtService::with_offset(0);
        assert_eq!(svc.format_time_context(None, TS), "时间未知");
    }

    #[test]
    fn default_offset_matches_system_date() {
        // 默认实例的本地偏移必须与系统 `date +%z` 一致(自洽断言,跨机器稳定)。
        let svc = TimefmtService::new();
        let rendered = svc.format_absolute(TS);
        let offset_part = rendered.rsplit(' ').next().expect("has offset");
        let expected = match offset_from_date_command() {
            Some(secs) => {
                let sign = if secs < 0 { '-' } else { '+' };
                let abs = secs.unsigned_abs();
                format!("{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60)
            }
            None => "+00:00".to_string(), // 与 system_local_offset_secs 的显式降级一致
        };
        assert_eq!(
            offset_part, expected,
            "default offset must match system local tz: {rendered}"
        );
    }

    #[test]
    fn iso_offset_parser() {
        assert_eq!(parse_iso_offset("+0800"), Some(28_800));
        assert_eq!(parse_iso_offset("-0530"), Some(-19_800));
        assert_eq!(parse_iso_offset("+08:00"), Some(28_800));
        assert_eq!(parse_iso_offset("+08"), Some(28_800));
        assert_eq!(parse_iso_offset("+0000"), Some(0));
        assert_eq!(parse_iso_offset("2400"), None); // 无符号
        assert_eq!(parse_iso_offset("+24:00"), None); // 越界
        assert_eq!(parse_iso_offset("+0860"), None); // 分钟越界
        assert_eq!(parse_iso_offset("garbage"), None);
        assert_eq!(parse_iso_offset(""), None);
    }

    #[test]
    fn posix_tz_env_parser() {
        // POSIX 符号:offset 是加到本地得 UTC 的加数;正 = 西,负 = 东。
        assert_eq!(posix_offset_secs("UTC"), Some(0));
        assert_eq!(posix_offset_secs("GMT"), Some(0));
        assert_eq!(posix_offset_secs("Etc/UTC"), Some(0));
        // "UTC+8" = 西八区(与 date +%z 输出 -0800 一致)。
        assert_eq!(posix_offset_secs("UTC+8"), Some(-28_800));
        // "CST-8" = 东八区(与 date +%z 输出 +0800 一致)。
        assert_eq!(posix_offset_secs("CST-8"), Some(28_800));
        // 纯偏移形式:POSIX 下 "+08:00" 同样表示西八区。
        assert_eq!(posix_offset_secs("+08:00"), Some(-28_800));
        assert_eq!(posix_offset_secs("-08:00"), Some(28_800));
        // 单数字小时与分钟级偏移。
        assert_eq!(posix_offset_secs("UTC+8"), Some(-28_800)); // 上面已断言,这里再锁一次
        assert_eq!(posix_offset_secs("EST5"), Some(-18_000)); // 西五区(美国东部)
        assert_eq!(posix_offset_secs("EST5EDT"), Some(-18_000)); // dst 后缀忽略
        assert_eq!(posix_offset_secs("GMT-0530"), Some(19_800)); // 东五区半
        // 命名区域(如 Asia/Shanghai)无法无数据库解析 → None,交给 date 命令。
        assert_eq!(posix_offset_secs("Asia/Shanghai"), None);
        assert_eq!(posix_offset_secs(""), None);
    }

    #[test]
    fn plugin_registers_timefmt() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(TimefmtPlugin::with_offset(28_800));
        let effects = ctx.mount(&plugin).expect("mount");
        let svc = ctx.service::<dyn Timefmt>(&TIMEFMT).expect("timefmt seam");
        assert_eq!(
            svc.format_time_context(Some(TS), TS + 180_000),
            "2026-05-27 14:30:05 +08:00 (3 分钟前)"
        );
        assert_eq!(
            svc.relative_key_and_value(42_000),
            RelativeBucket::new(KEY_SECONDS_AGO, Some(42))
        );
        drop(effects);
        assert!(!ctx.has_service(&TIMEFMT));
    }
}
