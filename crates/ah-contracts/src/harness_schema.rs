//! harness schema 停止条件(对齐 openjiuwen/harness/schema/stop_condition.py)。
//!
//! 确定性部分:StopEvaluationContext + 四个内置求值器(MaxRounds/TokenBudget/
//! Timeout/CompletionPromise)+ 序列化状态快照。CustomPredicate 依赖调用方闭包。

use serde_json::Value;

/// 停止评估上下文(对齐 StopEvaluationContext)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StopEvaluationContext {
    /// 已完成的外层循环轮数。
    #[serde(default)]
    pub iteration: u64,
    /// 全部轮次的累计 token 用量。
    #[serde(default)]
    pub token_usage: u64,
    /// 循环启动以来的墙钟秒数。
    #[serde(default)]
    pub elapsed_seconds: f64,
    /// 最近一轮的结果 dict。
    #[serde(default)]
    pub last_result: Option<Value>,
    /// 自定义求值器的任意扩展数据。
    #[serde(default)]
    pub extra: serde_json::Map<String, Value>,
}

impl Default for StopEvaluationContext {
    fn default() -> Self {
        Self {
            iteration: 0,
            token_usage: 0,
            elapsed_seconds: 0.0,
            last_result: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// 停止条件求值器状态快照(对齐 CompletionPromiseEvaluator.get_state/load_state)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompletionPromiseState {
    pub fulfilled: bool,
    pub matched_text: String,
    pub required_confirmations: u64,
    pub confirmation_count: u64,
}

impl Default for CompletionPromiseState {
    fn default() -> Self {
        Self {
            fulfilled: false,
            matched_text: String::new(),
            required_confirmations: 1,
            confirmation_count: 0,
        }
    }
}

/// 固定轮数停止(对齐 MaxRoundsEvaluator)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxRoundsEvaluator {
    pub max_rounds: u64,
}

impl MaxRoundsEvaluator {
    pub fn new(max_rounds: u64) -> Self {
        Self { max_rounds }
    }

    /// 完成轮数 >= max_rounds 即停(对齐 should_stop)。
    pub fn should_stop(&self, ctx: &StopEvaluationContext) -> bool {
        ctx.iteration >= self.max_rounds
    }
}

/// token 预算停止(对齐 TokenBudgetEvaluator)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudgetEvaluator {
    pub max_tokens: u64,
}

impl TokenBudgetEvaluator {
    pub fn new(max_tokens: u64) -> Self {
        Self { max_tokens }
    }

    pub fn should_stop(&self, ctx: &StopEvaluationContext) -> bool {
        ctx.token_usage >= self.max_tokens
    }
}

/// 墙钟超时停止(对齐 TimeoutEvaluator)。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutEvaluator {
    pub timeout_seconds: f64,
}

impl TimeoutEvaluator {
    pub fn new(timeout_seconds: f64) -> Self {
        Self { timeout_seconds }
    }

    pub fn should_stop(&self, ctx: &StopEvaluationContext) -> bool {
        ctx.elapsed_seconds >= self.timeout_seconds
    }
}

/// 完成承诺停止(对齐 CompletionPromiseEvaluator)。
///
/// 不直接解析 LLM 输出;TaskCompletionRail 检测到承诺标签后调用
/// `notify_fulfilled` 置位。确认计数是连续的:任何一次 `notify_absent`
/// 打断连续计数,需重新开始。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionPromiseEvaluator {
    pub promise: String,
    pub state: CompletionPromiseState,
}

impl CompletionPromiseEvaluator {
    pub fn new(promise: impl Into<String>, required_confirmations: u64) -> Self {
        Self {
            promise: promise.into(),
            state: CompletionPromiseState {
                required_confirmations: required_confirmations.max(1),
                ..Default::default()
            },
        }
    }

    /// 标记承诺已履行(对齐 notify_fulfilled):计数 +1,达要求即置位。
    pub fn notify_fulfilled(&mut self, matched_text: &str) {
        self.state.confirmation_count += 1;
        self.state.fulfilled = self.state.confirmation_count >= self.state.required_confirmations;
        self.state.matched_text = matched_text.to_string();
    }

    /// 记录承诺缺席(对齐 notify_absent):连续计数清零。
    pub fn notify_absent(&mut self) {
        self.state.confirmation_count = 0;
        self.state.fulfilled = false;
        self.state.matched_text = String::new();
    }

    /// 是否停止(对齐 should_stop):承诺标志置位。
    pub fn should_stop(&self, _ctx: &StopEvaluationContext) -> bool {
        self.state.fulfilled
    }

    /// 重置(对齐 reset)。
    pub fn reset(&mut self) {
        self.state = CompletionPromiseState {
            required_confirmations: self.state.required_confirmations,
            ..Default::default()
        };
    }

    /// 导出状态快照(对齐 get_state)。
    pub fn get_state(&self) -> CompletionPromiseState {
        self.state.clone()
    }

    /// 恢复状态(对齐 load_state)。
    pub fn load_state(&mut self, data: &Value) {
        let obj = data.as_object().cloned().unwrap_or_default();
        let mut fulfilled = obj
            .get("fulfilled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let matched_text = obj
            .get("matched_text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let required = obj
            .get("required_confirmations")
            .and_then(Value::as_u64)
            .unwrap_or(self.state.required_confirmations)
            .max(1);
        let mut confirmation_count = obj
            .get("confirmation_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        // 达要求的计数使 fulfilled 置位(或已有 fulfilled)。
        if confirmation_count >= required {
            fulfilled = true;
        }
        let _ = &mut confirmation_count;
        self.state.fulfilled = fulfilled;
        self.state.matched_text = matched_text;
        self.state.required_confirmations = required;
        self.state.confirmation_count = confirmation_count;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(iteration: u64, token_usage: u64, elapsed: f64) -> StopEvaluationContext {
        StopEvaluationContext {
            iteration,
            token_usage,
            elapsed_seconds: elapsed,
            ..Default::default()
        }
    }

    #[test]
    fn max_rounds_and_token_budget() {
        let mr = MaxRoundsEvaluator::new(3);
        assert!(!mr.should_stop(&ctx(2, 0, 0.0)));
        assert!(mr.should_stop(&ctx(3, 0, 0.0)));
        let tb = TokenBudgetEvaluator::new(1000);
        assert!(!tb.should_stop(&ctx(0, 999, 0.0)));
        assert!(tb.should_stop(&ctx(0, 1000, 0.0)));
    }

    #[test]
    fn timeout_evaluator() {
        let to = TimeoutEvaluator::new(30.0);
        assert!(!to.should_stop(&ctx(0, 0, 29.9)));
        assert!(to.should_stop(&ctx(0, 0, 30.0)));
    }

    #[test]
    fn completion_promise_confirmations() {
        let mut cp = CompletionPromiseEvaluator::new("<done>", 2);
        assert!(!cp.should_stop(&ctx(0, 0, 0.0)));
        cp.notify_fulfilled("x");
        assert!(!cp.should_stop(&ctx(0, 0, 0.0)), "need 2 confirmations");
        cp.notify_absent();
        assert!(!cp.should_stop(&ctx(0, 0, 0.0)), "absent breaks streak");
        cp.notify_fulfilled("x");
        cp.notify_fulfilled("y");
        assert!(cp.should_stop(&ctx(0, 0, 0.0)));
        assert_eq!(cp.state.matched_text, "y");
    }

    #[test]
    fn completion_promise_state_roundtrip() {
        let mut cp = CompletionPromiseEvaluator::new("<done>", 1);
        cp.notify_fulfilled("matched");
        let state = cp.get_state();
        assert!(state.fulfilled);
        assert_eq!(state.matched_text, "matched");

        let mut cp2 = CompletionPromiseEvaluator::new("<done>", 1);
        cp2.load_state(&json!({
            "fulfilled": true,
            "matched_text": "m",
            "required_confirmations": 3,
            "confirmation_count": 2
        }));
        // 对齐 Python:fulfilled = (count >= required) or 数据中的 fulfilled。
        assert!(cp2.state.fulfilled);
        assert_eq!(cp2.state.confirmation_count, 2);
        assert_eq!(cp2.state.required_confirmations, 3);
    }

    #[test]
    fn reset_clears_state_keeps_requirement() {
        let mut cp = CompletionPromiseEvaluator::new("<done>", 3);
        cp.notify_fulfilled("x");
        cp.notify_fulfilled("y");
        cp.notify_fulfilled("z");
        assert!(cp.should_stop(&ctx(0, 0, 0.0)));
        cp.reset();
        assert!(!cp.should_stop(&ctx(0, 0, 0.0)));
        assert_eq!(cp.state.required_confirmations, 3);
    }
}
