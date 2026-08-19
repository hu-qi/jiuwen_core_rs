//! 经验展示跟踪纯逻辑(对齐 experience/tracker.py 的确定性部分)。
//!
//! 纯逻辑:body 记录筛选 + 展示统计递增(times_presented+1,last_presented_at=now)+
//! 评估间隔消费计数;session/store/scorer 留待集成。

use crate::checkpoint_types::UsageStats;
use crate::experience_query::RecordView;

/// body 记录判定(对齐 _is_body_record:target == body)。
pub fn is_body_record(record: &RecordView) -> bool {
    record.target == "body"
}

/// 展示统计递增(对齐 record_presented 的 UsageStats 递增:presented+1,last_presented_at=now)。
pub fn increment_presented_stats(stats: &UsageStats, now: &str) -> UsageStats {
    UsageStats {
        times_presented: stats.times_presented + 1,
        times_used: stats.times_used,
        times_positive: stats.times_positive,
        times_negative: stats.times_negative,
        last_presented_at: Some(now.to_string()),
        last_evaluated_at: stats.last_evaluated_at.clone(),
    }
}

/// 选择 body 展示记录(对齐 record_presented:score ≥ min_score 且 body,取前 limit 条)。
pub fn select_body_records(
    records: &[RecordView],
    min_score: f64,
    limit: usize,
) -> Vec<RecordView> {
    records
        .iter()
        .filter(|r| r.score >= min_score && is_body_record(r))
        .take(limit)
        .cloned()
        .collect()
}

/// 评估间隔消费(对齐 consume_eval_state 的计数逻辑)。
///
/// 返回 (新计数, 是否到达间隔应消费):counter+1 >= interval → (0, true),否则 (counter+1, false)。
pub fn consume_eval_state(counter: usize, eval_interval: usize) -> (usize, bool) {
    if counter + 1 >= eval_interval {
        (0, true)
    } else {
        (counter + 1, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(target: &str, score: f64) -> RecordView {
        RecordView {
            id: "r".to_string(),
            summary: None,
            score,
            timestamp: "t".to_string(),
            target: target.to_string(),
            section: "S".to_string(),
            content: "c".to_string(),
        }
    }

    #[test]
    fn body_record_detection() {
        assert!(is_body_record(&rec("body", 0.5)));
        assert!(!is_body_record(&rec("description", 0.5)));
        assert!(!is_body_record(&rec("script", 0.5)));
    }

    #[test]
    fn presented_stats_increment() {
        let stats = UsageStats {
            times_presented: 2,
            times_used: 1,
            times_positive: 1,
            times_negative: 0,
            last_presented_at: Some("old".to_string()),
            last_evaluated_at: Some("eval".to_string()),
        };
        let updated = increment_presented_stats(&stats, "now");
        assert_eq!(updated.times_presented, 3);
        assert_eq!(updated.times_used, 1);
        assert_eq!(updated.times_positive, 1);
        assert_eq!(updated.last_presented_at.as_deref(), Some("now"));
        assert_eq!(updated.last_evaluated_at.as_deref(), Some("eval"));
    }

    #[test]
    fn select_body_records_filters_and_limits() {
        let records = vec![
            rec("body", 0.8),
            rec("body", 0.4), // 低于 min_score
            rec("description", 0.9),
            rec("body", 0.7),
            rec("body", 0.6),
        ];
        let selected = select_body_records(&records, 0.5, 2);
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].score, 0.8);
        assert_eq!(selected[1].score, 0.7);
    }

    #[test]
    fn eval_interval_consumption() {
        assert_eq!(consume_eval_state(0, 5), (1, false));
        assert_eq!(consume_eval_state(3, 5), (4, false));
        assert_eq!(consume_eval_state(4, 5), (0, true));
        assert_eq!(consume_eval_state(0, 1), (0, true));
    }
}
