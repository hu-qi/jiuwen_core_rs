//! # trajectory span codec(OTLP span codec + 聚合)
//!
//! 对齐 Python agent_evolving/trajectory:把 Trajectory 编码为 telemetry Span
//! 列表(OTel 树形结构),可逆解码,并聚合多条轨迹为统计。
//! 纯函数 + 真实数据往返,无 LLM 依赖。

use ah_contracts::evolving::{StepOutcome, Trajectory, TrajectoryStep};
use ah_contracts::telemetry::Span;
use serde_json::{Value, json};

/// 编码一条轨迹为 span 树:
/// - 根 span "trajectory/<task>",属性含 finished/steps 数;
/// - 每步一个子 span "step/<seq>",父为根,属性含 tool/outcome/error/budget。
pub fn trajectory_to_spans(trajectory: &Trajectory) -> Vec<Span> {
    let root_name = format!("trajectory/{}", trajectory.task);
    let mut attributes = serde_json::Map::new();
    attributes.insert("task".to_string(), json!(trajectory.task));
    attributes.insert("finished".to_string(), json!(trajectory.finished));
    attributes.insert("steps".to_string(), json!(trajectory.steps.len()));
    let root = Span {
        name: root_name.clone(),
        parent: None,
        attributes,
        start_ms: 0,
        duration_ms: 0,
    };
    let mut spans = vec![root];
    for (index, step) in trajectory.steps.iter().enumerate() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("action".to_string(), json!(step.action));
        attributes.insert("seq".to_string(), json!(step.seq));
        attributes.insert("tool".to_string(), json!(step.tool));
        attributes.insert("outcome".to_string(), json!(step.outcome));
        attributes.insert("error".to_string(), json!(step.error));
        attributes.insert("budget_used".to_string(), json!(step.budget_used));
        spans.push(Span {
            name: format!("step/{index}"),
            parent: Some(root_name.clone()),
            attributes,
            start_ms: step.seq,
            duration_ms: 1,
        });
    }
    spans
}

/// 解码 span 列表回轨迹(逆 [trajectory_to_spans];失败返回 None)。
pub fn spans_to_trajectory(spans: &[Span]) -> Option<Trajectory> {
    let root = spans.iter().find(|s| s.parent.is_none())?;
    let task = root.attributes.get("task")?.as_str()?.to_string();
    let finished = root
        .attributes
        .get("finished")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut steps: Vec<(usize, TrajectoryStep)> = spans
        .iter()
        .filter(|s| s.parent.is_some())
        .filter_map(|span| {
            let index = span.name.strip_prefix("step/")?.parse::<usize>().ok()?;
            let seq = span.attributes.get("seq")?.as_u64()?;
            let action = span
                .attributes
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let tool = span
                .attributes
                .get("tool")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let outcome = span
                .attributes
                .get("outcome")
                .and_then(|v| serde_json::from_value::<StepOutcome>(v.clone()).ok())
                .unwrap_or(StepOutcome::Skipped);
            let error = span
                .attributes
                .get("error")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let budget_used = span
                .attributes
                .get("budget_used")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32;
            Some((
                index,
                TrajectoryStep {
                    seq,
                    action,
                    tool,
                    outcome,
                    error,
                    budget_used,
                },
            ))
        })
        .collect();
    steps.sort_by_key(|(index, _)| *index);
    Some(Trajectory {
        task,
        steps: steps.into_iter().map(|(_, step)| step).collect(),
        finished,
    })
}

/// 轨迹聚合统计。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryStats {
    /// 轨迹总数。
    pub total: usize,
    /// 预算内完成数。
    pub finished: usize,
    /// 总步数。
    pub steps: usize,
    /// 错误步数。
    pub errors: usize,
    /// 平均步数。
    pub avg_steps: f64,
    /// 完成率(0..1)。
    pub finish_rate: f64,
}

/// 聚合多条轨迹(真实统计:完成/错误/平均步数/完成率)。
pub fn aggregate_trajectories(trajectories: &[Trajectory]) -> TrajectoryStats {
    let total = trajectories.len();
    let finished = trajectories.iter().filter(|t| t.finished).count();
    let steps: usize = trajectories.iter().map(|t| t.steps.len()).sum();
    let errors: usize = trajectories
        .iter()
        .flat_map(|t| t.steps.iter())
        .filter(|s| s.outcome == StepOutcome::Error)
        .count();
    TrajectoryStats {
        total,
        finished,
        steps,
        errors,
        avg_steps: if total == 0 {
            0.0
        } else {
            steps as f64 / total as f64
        },
        finish_rate: if total == 0 {
            0.0
        } else {
            finished as f64 / total as f64
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trajectory() -> Trajectory {
        Trajectory {
            task: "build feature".to_string(),
            finished: true,
            steps: vec![
                TrajectoryStep {
                    seq: 0,
                    action: "list workspace".to_string(),
                    tool: Some("list_dir".to_string()),
                    outcome: StepOutcome::Success,
                    error: None,
                    budget_used: 1,
                },
                TrajectoryStep {
                    seq: 1,
                    action: "read config".to_string(),
                    tool: Some("read_file".to_string()),
                    outcome: StepOutcome::Error,
                    error: Some("file not found".to_string()),
                    budget_used: 1,
                },
            ],
        }
    }

    #[test]
    fn trajectory_span_roundtrip_is_exact() {
        let trajectory = sample_trajectory();
        let spans = trajectory_to_spans(&trajectory);
        assert_eq!(spans.len(), 3, "root + 2 steps");
        assert_eq!(spans[0].name, "trajectory/build feature");
        assert!(spans[0].parent.is_none(), "root has no parent");
        assert_eq!(spans[1].parent.as_deref(), Some("trajectory/build feature"));
        assert_eq!(spans[1].attributes["tool"], json!("list_dir"));
        assert_eq!(spans[2].attributes["error"], json!("file not found"));

        let decoded = spans_to_trajectory(&spans).expect("decode");
        assert_eq!(decoded, trajectory, "roundtrip exact");
    }

    #[test]
    fn aggregation_counts_real_statistics() {
        let a = sample_trajectory();
        let mut b = sample_trajectory();
        b.finished = false;
        b.steps = vec![TrajectoryStep {
            seq: 0,
            action: "boom".to_string(),
            tool: None,
            outcome: StepOutcome::Error,
            error: Some("crash".to_string()),
            budget_used: 1,
        }];
        let stats = aggregate_trajectories(&[a, b]);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.finished, 1);
        assert_eq!(stats.finish_rate, 0.5);
        assert_eq!(stats.steps, 3, "2 + 1");
        assert_eq!(stats.errors, 2, "one error per trajectory");
        assert!((stats.avg_steps - 1.5).abs() < 1e-9);

        let empty = aggregate_trajectories(&[]);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.finish_rate, 0.0);
    }
}
