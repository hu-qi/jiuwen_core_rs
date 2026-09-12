//! runner seam:回调链框架(对齐 Python core/runner/callback)。
//!
//! 回调链按优先级顺序执行回调,支持 retry / timeout / break / rollback:
//! - CONTINUE:继续下一个;
//! - BREAK:短路并返回当前结果;
//! - RETRY:重试当前回调(受 max_retries 限制);
//! - ROLLBACK:逆序执行已执行回调的 rollback handler 后返回。
//!
//! CallbackMetrics 记录调用次数/耗时/错误率(可观测)。

use async_trait::async_trait;
use serde_json::Value;

use crate::seam::Seam;

/// 链动作(对齐 ChainAction)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainAction {
    Continue,
    Break,
    Retry,
    Rollback,
}

/// 回调执行指标(对齐 CallbackMetrics)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CallbackMetrics {
    pub call_count: u64,
    pub total_ms: u64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub error_count: u64,
    pub last_call_ms: u64,
}

impl Default for CallbackMetrics {
    fn default() -> Self {
        Self {
            call_count: 0,
            total_ms: 0,
            min_ms: u64::MAX,
            max_ms: 0,
            error_count: 0,
            last_call_ms: 0,
        }
    }
}

impl CallbackMetrics {
    /// 记录一次执行。
    pub fn update(&mut self, elapsed_ms: u64, is_error: bool) {
        self.call_count += 1;
        self.total_ms += elapsed_ms;
        self.min_ms = self.min_ms.min(elapsed_ms);
        self.max_ms = self.max_ms.max(elapsed_ms);
        self.last_call_ms = elapsed_ms;
        if is_error {
            self.error_count += 1;
        }
    }

    /// 平均耗时(无调用返回 0)。
    pub fn avg_ms(&self) -> u64 {
        self.total_ms.checked_div(self.call_count).unwrap_or(0)
    }

    /// 错误率(0.0..=1.0)。
    pub fn error_rate(&self) -> f64 {
        if self.call_count == 0 {
            0.0
        } else {
            self.error_count as f64 / self.call_count as f64
        }
    }
}

/// rollback handler(错误/ROLLBACK 动作时逆序执行)。
pub type RollbackHandler = std::sync::Arc<dyn Fn(&ChainContext) + Send + Sync>;

/// 回调执行上下文(链内共享)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChainContext {
    /// 初始参数。
    pub initial_args: Vec<Value>,
    /// 已执行回调的结果。
    pub results: Vec<Value>,
}

impl ChainContext {
    /// 上一个结果(无则 None)。
    pub fn last_result(&self) -> Option<Value> {
        self.results.last().cloned()
    }
}

/// 回调返回。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChainResult {
    pub action: ChainAction,
    pub result: Value,
}

impl ChainResult {
    pub fn continue_(result: Value) -> Self {
        Self {
            action: ChainAction::Continue,
            result,
        }
    }

    pub fn break_(result: Value) -> Self {
        Self {
            action: ChainAction::Break,
            result,
        }
    }

    pub fn retry() -> Self {
        Self {
            action: ChainAction::Retry,
            result: Value::Null,
        }
    }

    pub fn rollback() -> Self {
        Self {
            action: ChainAction::Rollback,
            result: Value::Null,
        }
    }
}

/// runner 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerError(pub String);

impl core::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RunnerError {}

/// 单个回调(消费方实现真实逻辑)。
#[async_trait]
pub trait ChainCallback: Send + Sync {
    /// 回调名(可逆注册去重)。
    fn name(&self) -> &'static str;

    /// 执行回调,返回动作与结果。
    async fn call(&self, context: &ChainContext) -> Result<ChainResult, RunnerError>;
}

/// 回调链 Seam(Service Definition):按优先级顺序执行 + 回滚。
#[async_trait]
pub trait CallbackChain: Seam {
    /// 追加回调(含优先级/启用/重试/超时);可逆。
    fn add(
        &self,
        callback: std::sync::Arc<dyn ChainCallback>,
        priority: i32,
        enabled: bool,
        max_retries: u32,
        timeout_ms: Option<u64>,
        rollback: Option<RollbackHandler>,
    ) -> crate::effect::Effect;

    /// 移除回调。
    fn remove(&self, name: &str);

    /// 执行链(按优先级降序;retry/break/rollback 语义)。
    async fn execute(&self, initial_args: Vec<Value>) -> Result<ChainResult, RunnerError>;

    /// 当前指标。
    fn metrics(&self) -> CallbackMetrics;

    /// 已注册回调名(按优先级降序)。
    fn names(&self) -> Vec<String>;
}
