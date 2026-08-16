//! # ah-plugins-experience-scorer
//!
//! 真实经验评分(对齐 openjiuwen/agent_evolving/experience/scorer.py 的确定性核心):
//! - E(有效性):贝叶斯平滑 (positive+1)/(total+2),无数据时中性 0.5;
//! - U(利用率):used/presented,无数据时中性 0.5;
//! - F(新鲜度):指数衰减 0.5 + 0.5·2^(-days/90),范围 [0.5,1.0];版本过期 ×0.7;
//! - 总分 = 0.5·E + 0.3·U + 0.2·F;
//! - update:按评估结果(used/positive/negative)累加统计、记录评估时间并重算分数。
//!
//! 时间处理不依赖外部 crate:手写 ISO-8601 解析(UTC)与 civil→days 转换,
//! 天数差用 floor 除法(对齐 Python timedelta.days)。LLM 评估与库整理留待后续。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ah_contracts::keys::EXPERIENCE_SCORER;
use ah_contracts::prelude::Effect;
use ah_contracts::scoring::{ExperienceScorer, ScoredExperience, ScorerError, UsageStats};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// E/U/F 权重(对齐 W_E/W_U/W_F)。
pub const W_E: f64 = 0.5;
pub const W_U: f64 = 0.3;
pub const W_F: f64 = 0.2;

/// 新鲜度半衰期(天)。
pub const FRESHNESS_HALF_LIFE_DAYS: f64 = 90.0;
/// 版本过期惩罚系数。
pub const STALE_VERSION_PENALTY: f64 = 0.7;

/// civil date → days since 1970-01-01(Howard Hinnant 算法)。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = ((m + 9) % 12) as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 把 (年,月,日,时,分,秒) 转成 UTC epoch 秒。
fn civil_to_epoch(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    days_from_civil(y, mo, d) * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64
}

/// epoch 秒 → UTC ISO-8601 "YYYY-MM-DDTHH:MM:SSZ"。
pub fn format_iso(secs: i64) -> String {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;
    // days → civil(逆算法)。
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// 解析 ISO-8601(UTC)为 epoch 秒;支持 "YYYY-MM-DD" 与 "YYYY-MM-DDTHH:MM:SS[.fff][Z|+hh:mm]"。
/// 解析失败返回 None。
fn parse_iso_epoch(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.len() < 10 {
        return None;
    }
    let y: i64 = text.get(0..4)?.parse().ok()?;
    if text.get(4..5)? != "-" {
        return None;
    }
    let mo: u32 = text.get(5..7)?.parse().ok()?;
    if text.get(7..8)? != "-" {
        return None;
    }
    let d: u32 = text.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    if text.len() == 10 {
        return Some(civil_to_epoch(y, mo, d, 0, 0, 0));
    }
    // 时间部分(可选):T 或空格后 HH:MM:SS。
    let rest = &text[10..];
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    if rest.len() < 8 {
        return None;
    }
    let h: u32 = rest.get(0..2)?.parse().ok()?;
    if rest.get(2..3)? != ":" {
        return None;
    }
    let mi: u32 = rest.get(3..5)?.parse().ok()?;
    if rest.get(5..6)? != ":" {
        return None;
    }
    let s: u32 = rest.get(6..8)?.parse().ok()?;
    if h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let mut secs = civil_to_epoch(y, mo, d, h, mi, s);
    let tail = &rest[8..];
    // 小数秒:忽略。
    let tail = match tail.strip_prefix('.') {
        Some(t) => {
            let idx = t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len());
            &t[idx..]
        }
        None => tail,
    };
    // 时区偏移:Z / +hh:mm / +hhmm / -hh:mm;naive(无偏移)视为 UTC。
    if let Some(t) = tail.strip_prefix(['Z', 'z']) {
        let _ = t;
    } else if let Some(t) = tail.strip_prefix(['+', '-']) {
        let sign = if tail.starts_with('-') { -1 } else { 1 };
        let digits = t;
        let (oh, om) = if digits.len() >= 5 && digits.get(2..3) == Some(":") {
            (
                digits.get(0..2)?.parse::<i64>().ok()?,
                digits.get(3..5)?.parse::<i64>().ok()?,
            )
        } else if digits.len() >= 4 {
            (
                digits.get(0..2)?.parse::<i64>().ok()?,
                digits.get(2..4)?.parse::<i64>().ok()?,
            )
        } else {
            return None;
        };
        secs -= sign * (oh * 3600 + om * 60);
    }
    Some(secs)
}

/// 当前 UTC epoch 秒。
fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 确定性经验评分器。
pub struct DeterministicExperienceScorer;

impl Seam for DeterministicExperienceScorer {}

impl ExperienceScorer for DeterministicExperienceScorer {
    fn effectiveness(&self, stats: &UsageStats) -> f64 {
        let total = stats.times_positive + stats.times_negative;
        if total == 0 {
            return 0.5; // 无数据,中性。
        }
        // Beta(1,1) 先验:(positive + 1) / (total + 2)。
        (stats.times_positive as f64 + 1.0) / (total as f64 + 2.0)
    }

    fn utilization(&self, stats: &UsageStats) -> f64 {
        if stats.times_presented == 0 {
            return 0.5; // 无数据,中性。
        }
        stats.times_used as f64 / stats.times_presented as f64
    }

    fn freshness(
        &self,
        timestamp: Option<&str>,
        record_skill_version: Option<&str>,
        current_skill_version: Option<&str>,
    ) -> f64 {
        let Some(ts) = timestamp else {
            return 0.5;
        };
        let Some(record_epoch) = parse_iso_epoch(ts) else {
            return 0.5;
        };
        let days_old = (now_epoch() - record_epoch).div_euclid(86400);
        // 指数衰减:0.5·2^(-days/half_life)。
        let decay = 0.5 * 2.0_f64.powf(-(days_old as f64) / FRESHNESS_HALF_LIFE_DAYS);
        let mut freshness = 0.5 + decay;
        // 版本过期惩罚。
        if let (Some(cur), Some(rec)) = (current_skill_version, record_skill_version)
            && rec != cur
        {
            freshness *= STALE_VERSION_PENALTY;
        }
        freshness.clamp(0.0, 1.0)
    }

    fn score(&self, record: &ScoredExperience, current_skill_version: Option<&str>) -> f64 {
        let e = self.effectiveness(&record.usage_stats);
        let u = self.utilization(&record.usage_stats);
        let f = self.freshness(
            record.timestamp.as_deref(),
            record.skill_version.as_deref(),
            current_skill_version,
        );
        W_E * e + W_U * u + W_F * f
    }

    fn update(
        &self,
        record: &mut ScoredExperience,
        used: bool,
        positive: bool,
        negative: bool,
        current_skill_version: Option<&str>,
    ) -> Result<f64, ScorerError> {
        if used {
            record.usage_stats.times_used += 1;
        }
        if positive {
            record.usage_stats.times_positive += 1;
        }
        if negative {
            record.usage_stats.times_negative += 1;
        }
        record.usage_stats.last_evaluated_at = Some(format_iso(now_epoch()));
        let score = self.score(record, current_skill_version);
        record.score = score;
        Ok(score)
    }
}

/// 经验评分插件:注册 `experience-scorer` seam。
pub struct ExperienceScorerPlugin;

impl Plugin for ExperienceScorerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-experience-scorer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![EXPERIENCE_SCORER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let scorer: Arc<dyn ExperienceScorer> = Arc::new(DeterministicExperienceScorer);
        Ok(vec![ctx.register(EXPERIENCE_SCORER, scorer)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::EXPERIENCE_SCORER;
    use ah_hub::plugin::DynPlugin;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    #[test]
    fn effectiveness_bayesian_smoothing() {
        let scorer = DeterministicExperienceScorer;
        let empty = UsageStats::default();
        assert!(
            (scorer.effectiveness(&empty) - 0.5).abs() < 1e-9,
            "无数据中性 0.5"
        );

        let balanced = UsageStats {
            times_positive: 1,
            times_negative: 1,
            ..Default::default()
        };
        // (1+1)/(2+2) = 0.5。
        assert!((scorer.effectiveness(&balanced) - 0.5).abs() < 1e-9);

        let good = UsageStats {
            times_positive: 9,
            times_negative: 1,
            ..Default::default()
        };
        // (9+1)/(10+2) = 0.8333...
        assert!((scorer.effectiveness(&good) - 10.0 / 12.0).abs() < 1e-9);
    }

    #[test]
    fn utilization_ratio_and_neutral() {
        let scorer = DeterministicExperienceScorer;
        let no_data = UsageStats::default();
        assert!(
            (scorer.utilization(&no_data) - 0.5).abs() < 1e-9,
            "无展示中性"
        );

        let used_3_of_10 = UsageStats {
            times_used: 3,
            times_presented: 10,
            ..Default::default()
        };
        assert!((scorer.utilization(&used_3_of_10) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn freshness_time_decay() {
        let scorer = DeterministicExperienceScorer;
        // 无时间戳 → 中性。
        assert!((scorer.freshness(None, None, None) - 0.5).abs() < 1e-9);
        // 无效时间戳 → 中性。
        assert!((scorer.freshness(Some("not-a-date"), None, None) - 0.5).abs() < 1e-9);

        let n = now();
        // 今天 → days_old=0 → 0.5 + 0.5 = 1.0。
        assert!((scorer.freshness(Some(&format_iso(n)), None, None) - 1.0).abs() < 1e-9);
        // 90 天前 → 0.5 + 0.5·2^-1 = 0.75。
        assert!(
            (scorer.freshness(Some(&format_iso(n - 90 * 86400)), None, None) - 0.75).abs() < 1e-9
        );
        // 180 天前 → 0.5 + 0.5·2^-2 = 0.625。
        assert!(
            (scorer.freshness(Some(&format_iso(n - 180 * 86400)), None, None) - 0.625).abs() < 1e-9
        );
        // 未来时间戳 → 0.5 + 0.5 = 1.0(clamp)。
        assert!((scorer.freshness(Some(&format_iso(n + 86400)), None, None) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn freshness_version_staleness_penalty() {
        let scorer = DeterministicExperienceScorer;
        let n = now();
        // 版本一致 → 无惩罚。
        let same = scorer.freshness(Some(&format_iso(n)), Some("v2"), Some("v2"));
        assert!((same - 1.0).abs() < 1e-9);
        // 版本过期 → ×0.7。
        let stale = scorer.freshness(Some(&format_iso(n)), Some("v1"), Some("v2"));
        assert!((stale - 0.7).abs() < 1e-9, "stale={stale}");
        // 仅一端有版本 → 无惩罚。
        let one_side = scorer.freshness(Some(&format_iso(n)), Some("v1"), None);
        assert!((one_side - 1.0).abs() < 1e-9);
    }

    #[test]
    fn weighted_score_composition() {
        let scorer = DeterministicExperienceScorer;
        let n = now();
        let mut record = ScoredExperience::new("r1");
        record.timestamp = Some(format_iso(n)); // F = 1.0
        // E = 0.5(无数据),U = 0.3(3/10),F = 1.0。
        record.usage_stats.times_used = 3;
        record.usage_stats.times_presented = 10;
        let expected = 0.5 * 0.5 + 0.3 * 0.3 + 0.2 * 1.0; // 0.54
        let score = scorer.score(&record, None);
        assert!(
            (score - expected).abs() < 1e-9,
            "score={score} expected={expected}"
        );
    }

    #[test]
    fn update_increments_stats_and_recomputes() {
        let scorer = DeterministicExperienceScorer;
        let n = now();
        let mut record = ScoredExperience::new("r2");
        record.timestamp = Some(format_iso(n));
        let score = scorer
            .update(&mut record, true, true, false, None)
            .expect("update");
        assert_eq!(record.usage_stats.times_used, 1);
        assert_eq!(record.usage_stats.times_positive, 1);
        assert_eq!(record.usage_stats.times_negative, 0);
        assert!(
            record.usage_stats.last_evaluated_at.is_some(),
            "评估时间已记录"
        );
        // E = (1+1)/(1+2) = 2/3,U = 1/1 = 1.0(presented=0 → 0.5? presented=0 → 0.5)。
        // 注意:presented=0 → U 中性 0.5。
        let expected = 0.5 * (2.0 / 3.0) + 0.3 * 0.5 + 0.2 * 1.0;
        assert!((score - expected).abs() < 1e-9, "score={score}");
        assert!((record.score - score).abs() < 1e-9, "record.score 已更新");
    }

    #[test]
    fn iso_parse_and_format_roundtrip() {
        // 固定时间点往返。
        for secs in [0i64, 951782400, 1_700_000_000, 2_000_000_000] {
            let iso = format_iso(secs);
            let parsed = parse_iso_epoch(&iso).expect("parse");
            assert_eq!(parsed, secs, "{iso} 往返一致");
        }
        // 纯日期。
        let parsed = parse_iso_epoch("2026-01-15").expect("date");
        assert_eq!(parsed, civil_to_epoch(2026, 1, 15, 0, 0, 0));
        // Z 后缀与偏移。
        assert_eq!(
            parse_iso_epoch("2026-01-15T10:30:00Z").expect("Z"),
            parse_iso_epoch("2026-01-15T10:30:00").expect("naive")
        );
        let utc = parse_iso_epoch("2026-01-15T10:30:00Z").unwrap();
        let plus8 = parse_iso_epoch("2026-01-15T18:30:00+08:00").unwrap();
        assert_eq!(utc, plus8, "+08:00 转 UTC");
    }

    #[test]
    fn plugin_registers_scorer() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ExperienceScorerPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let scorer = ctx
            .service::<dyn ExperienceScorer>(&EXPERIENCE_SCORER)
            .expect("experience-scorer seam");
        assert!((scorer.effectiveness(&UsageStats::default()) - 0.5).abs() < 1e-9);
        drop(effects);
        assert!(!ctx.has_service(&EXPERIENCE_SCORER));
    }
}
