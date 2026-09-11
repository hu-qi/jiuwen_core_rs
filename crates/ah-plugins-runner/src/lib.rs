//! # ah-plugins-runner
//!
//! 真实回调链(对齐 Python core/runner/callback/chain.py):
//! - 按优先级降序执行;
//! - 单回调 retry(max_retries 内);
//! - 可选 timeout(超时判错);
//! - BREAK 短路;ROLLBACK 逆序执行已成功回调的 rollback handler;
//! - CallbackMetrics 记录调用次数/耗时/错误率。
//!
//! 注册可逆(Effect drop 即移除)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ah_contracts::effect::Effect;
use ah_contracts::keys::RUNNER;
use ah_contracts::runner::{
    CallbackChain, CallbackMetrics, ChainAction, ChainCallback, ChainContext, ChainResult,
    RollbackHandler, RunnerError,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::Value;

/// 链内回调条目。
#[derive(Clone)]
struct CallbackEntry {
    callback: Arc<dyn ChainCallback>,
    priority: i32,
    enabled: bool,
    max_retries: u32,
    timeout_ms: Option<u64>,
    rollback: Option<RollbackHandler>,
}

/// 真实回调链。
pub struct LocalCallbackChain {
    /// name → entry。
    callbacks: Arc<Mutex<HashMap<String, CallbackEntry>>>,
    metrics: Mutex<CallbackMetrics>,
}

impl LocalCallbackChain {
    pub fn new() -> Self {
        Self {
            callbacks: Arc::new(Mutex::new(HashMap::new())),
            metrics: Mutex::new(CallbackMetrics::default()),
        }
    }

    fn sorted_entries(&self) -> Vec<(String, CallbackEntry)> {
        let map = self.callbacks.lock().unwrap();
        let mut entries: Vec<(String, CallbackEntry)> = map
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    CallbackEntry {
                        callback: entry.callback.clone(),
                        priority: entry.priority,
                        enabled: entry.enabled,
                        max_retries: entry.max_retries,
                        timeout_ms: entry.timeout_ms,
                        rollback: entry.rollback.clone(),
                    },
                )
            })
            .collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.1.priority));
        entries
    }

    fn record(&self, elapsed_ms: u64, is_error: bool) {
        self.metrics.lock().unwrap().update(elapsed_ms, is_error);
    }
}

impl Default for LocalCallbackChain {
    fn default() -> Self {
        Self::new()
    }
}

impl Seam for LocalCallbackChain {}

#[async_trait]
impl CallbackChain for LocalCallbackChain {
    fn add(
        &self,
        callback: Arc<dyn ChainCallback>,
        priority: i32,
        enabled: bool,
        max_retries: u32,
        timeout_ms: Option<u64>,
        rollback: Option<RollbackHandler>,
    ) -> Effect {
        let name = callback.name().to_string();
        self.callbacks.lock().unwrap().insert(
            name.clone(),
            CallbackEntry {
                callback,
                priority,
                enabled,
                max_retries,
                timeout_ms,
                rollback,
            },
        );
        let callbacks = self.callbacks.clone();
        Effect::new(move || {
            callbacks.lock().unwrap().remove(&name);
        })
    }

    fn remove(&self, name: &str) {
        self.callbacks.lock().unwrap().remove(name);
    }

    async fn execute(&self, initial_args: Vec<Value>) -> Result<ChainResult, RunnerError> {
        let mut context = ChainContext {
            initial_args,
            results: Vec::new(),
        };
        let entries = self.sorted_entries();
        // 已成功执行的回调(rollback 逆序)。
        let mut executed: Vec<CallbackEntry> = Vec::new();

        for entry in entries.iter().filter(|(_, e)| e.enabled) {
            let (name, entry) = (entry.0.clone(), entry.1.clone());
            let mut attempt = 0;
            loop {
                let started = Instant::now();
                let outcome = async {
                    let args = if context.results.is_empty() {
                        context.initial_args.clone()
                    } else {
                        let mut args = vec![context.last_result().unwrap_or(Value::Null)];
                        args.extend(context.initial_args.clone());
                        args
                    };
                    let call_context = ChainContext {
                        initial_args: args,
                        results: context.results.clone(),
                    };
                    entry.callback.call(&call_context).await
                };
                let result = if let Some(timeout_ms) = entry.timeout_ms {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        outcome,
                    )
                    .await
                    {
                        Ok(inner) => inner,
                        Err(_) => Err(RunnerError(format!(
                            "callback {name} timed out after {timeout_ms}ms"
                        ))),
                    }
                } else {
                    outcome.await
                };

                match result {
                    Ok(chain_result) => {
                        let elapsed = started.elapsed().as_millis() as u64;
                        self.record(elapsed, false);
                        match chain_result.action {
                            ChainAction::Break => {
                                context.results.push(chain_result.result);
                                return Ok(ChainResult {
                                    action: ChainAction::Break,
                                    result: context.last_result().unwrap_or(Value::Null),
                                });
                            }
                            ChainAction::Retry => {
                                if attempt >= entry.max_retries {
                                    return Err(RunnerError(format!(
                                        "callback {name} retried {attempt} times and still requested retry"
                                    )));
                                }
                                attempt += 1;
                                continue;
                            }
                            ChainAction::Rollback => {
                                // 逆序执行已成功回调的 rollback。
                                for done in executed.iter().rev() {
                                    if let Some(rollback) = &done.rollback {
                                        rollback(&context);
                                    }
                                }
                                return Ok(ChainResult {
                                    action: ChainAction::Rollback,
                                    result: Value::Null,
                                });
                            }
                            ChainAction::Continue => {
                                context.results.push(chain_result.result);
                                executed.push(CallbackEntry {
                                    callback: entry.callback.clone(),
                                    priority: entry.priority,
                                    enabled: entry.enabled,
                                    max_retries: entry.max_retries,
                                    timeout_ms: entry.timeout_ms,
                                    rollback: entry.rollback.clone(),
                                });
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        let elapsed = started.elapsed().as_millis() as u64;
                        self.record(elapsed, true);
                        // 错误时也回滚已执行回调。
                        for done in executed.iter().rev() {
                            if let Some(rollback) = &done.rollback {
                                rollback(&context);
                            }
                        }
                        return Err(RunnerError(format!("callback {name} failed: {}", error.0)));
                    }
                }
            }
        }
        Ok(ChainResult {
            action: ChainAction::Continue,
            result: context.last_result().unwrap_or(Value::Null),
        })
    }

    fn metrics(&self) -> CallbackMetrics {
        self.metrics.lock().unwrap().clone()
    }

    fn names(&self) -> Vec<String> {
        self.sorted_entries()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }
}

/// runner 插件:提供 callback-chain seam。
pub struct RunnerPlugin;

impl Plugin for RunnerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-runner"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![RUNNER]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let chain: Arc<dyn CallbackChain> = Arc::new(LocalCallbackChain::new());
        Ok(vec![ctx.register(RUNNER, chain)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::RUNNER;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(RunnerPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    /// 记录调用的回调。
    struct NamedCallback {
        name: &'static str,
        action: ChainAction,
        value: &'static str,
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ChainCallback for NamedCallback {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn call(&self, context: &ChainContext) -> Result<ChainResult, RunnerError> {
            self.log.lock().unwrap().push(format!(
                "{}:{}",
                self.name,
                context.last_result().unwrap_or(Value::Null)
            ));
            match self.action {
                ChainAction::Continue => Ok(ChainResult::continue_(json!({ "v": self.value }))),
                ChainAction::Break => Ok(ChainResult::break_(json!({ "v": self.value }))),
                ChainAction::Retry => Ok(ChainResult::retry()),
                ChainAction::Rollback => Ok(ChainResult::rollback()),
            }
        }
    }

    #[tokio::test]
    async fn executes_in_priority_order_and_chains_results() {
        let (ctx, effects) = build_ctx();
        let chain = ctx.service::<dyn CallbackChain>(&RUNNER).expect("chain");
        let log = Arc::new(Mutex::new(Vec::new()));

        let low = Arc::new(NamedCallback {
            name: "low",
            action: ChainAction::Continue,
            value: "low-result",
            log: log.clone(),
        });
        let high = Arc::new(NamedCallback {
            name: "high",
            action: ChainAction::Continue,
            value: "high-result",
            log: log.clone(),
        });
        // 先加 low,再加 high → high(优先级高)先执行。
        let _guards = [
            chain.add(low, 1, true, 0, None, None),
            chain.add(high, 10, true, 0, None, None),
        ];

        let result = chain
            .execute(vec![json!({"seed": 1})])
            .await
            .expect("execute");
        assert_eq!(result.action, ChainAction::Continue);
        assert_eq!(
            result.result["v"], "low-result",
            "low runs last -> its result is final"
        );
        let order = log.lock().unwrap().clone();
        assert_eq!(order.len(), 2);
        assert!(order[0].starts_with("high:"), "high runs first: {order:?}");
        assert!(order[1].starts_with("low:"), "low runs second: {order:?}");

        // 链式传递:low 收到 high 的结果。
        assert!(order[1].contains("high-result"), "chained: {order:?}");
        assert_eq!(chain.names(), vec!["high", "low"], "priority order names");

        let metrics = chain.metrics();
        assert_eq!(metrics.call_count, 2);
        assert_eq!(metrics.error_count, 0);

        drop(effects);
    }

    #[tokio::test]
    async fn break_short_circuits_and_retry_limits() {
        let (ctx, effects) = build_ctx();
        let chain = ctx.service::<dyn CallbackChain>(&RUNNER).expect("chain");
        let log = Arc::new(Mutex::new(Vec::new()));

        let breaker = Arc::new(NamedCallback {
            name: "breaker",
            action: ChainAction::Break,
            value: "stop-here",
            log: log.clone(),
        });
        let after = Arc::new(NamedCallback {
            name: "after",
            action: ChainAction::Continue,
            value: "never",
            log: log.clone(),
        });
        let mut guards = vec![
            chain.add(breaker, 5, true, 0, None, None),
            chain.add(after, 1, true, 0, None, None),
        ];

        let result = chain.execute(vec![]).await.expect("execute");
        assert_eq!(result.action, ChainAction::Break);
        assert_eq!(result.result["v"], "stop-here");
        let order = log.lock().unwrap().clone();
        assert_eq!(order.len(), 1, "after must not run: {order:?}");

        // 重试:max_retries=0 → 一次 RETRY 即报错。
        let retryer = Arc::new(NamedCallback {
            name: "retryer",
            action: ChainAction::Retry,
            value: "x",
            log: Arc::new(Mutex::new(Vec::new())),
        });
        guards.push(chain.add(retryer, 9, true, 0, None, None));
        assert!(
            chain.execute(vec![]).await.is_err(),
            "retry beyond limit errors"
        );

        drop(effects);
    }

    #[tokio::test]
    async fn timeout_errors_and_missing_callback_errors() {
        let (ctx, effects) = build_ctx();
        let chain = ctx.service::<dyn CallbackChain>(&RUNNER).expect("chain");

        // 超时回调。
        struct Slow;
        #[async_trait]
        impl ChainCallback for Slow {
            fn name(&self) -> &'static str {
                "slow"
            }
            async fn call(&self, _ctx: &ChainContext) -> Result<ChainResult, RunnerError> {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                Ok(ChainResult::continue_(json!({"done": true})))
            }
        }
        let _slow_guard = chain.add(Arc::new(Slow), 5, true, 0, Some(50), None);
        let error = chain.execute(vec![]).await.expect_err("timeout");
        assert!(error.0.contains("timed out"));

        let metrics = chain.metrics();
        assert_eq!(metrics.error_count, 1, "timeout counted as error");
        assert!(metrics.avg_ms() > 0);

        drop(effects);
    }

    #[tokio::test]
    async fn rollback_runs_reverse_on_rollback_action_and_error() {
        let (ctx, effects) = build_ctx();
        let chain = ctx.service::<dyn CallbackChain>(&RUNNER).expect("chain");

        // 两个带 rollback 的回调 + 一个失败回调。
        let rollbacks = Arc::new(Mutex::new(Vec::new()));
        let rb = rollbacks.clone();
        struct Ok1;
        #[async_trait]
        impl ChainCallback for Ok1 {
            fn name(&self) -> &'static str {
                "ok1"
            }
            async fn call(&self, _ctx: &ChainContext) -> Result<ChainResult, RunnerError> {
                Ok(ChainResult::continue_(json!({"step": 1})))
            }
        }
        struct Ok2;
        #[async_trait]
        impl ChainCallback for Ok2 {
            fn name(&self) -> &'static str {
                "ok2"
            }
            async fn call(&self, _ctx: &ChainContext) -> Result<ChainResult, RunnerError> {
                Ok(ChainResult::rollback())
            }
        }
        struct Failing;
        #[async_trait]
        impl ChainCallback for Failing {
            fn name(&self) -> &'static str {
                "failing"
            }
            async fn call(&self, _ctx: &ChainContext) -> Result<ChainResult, RunnerError> {
                Err(RunnerError("boom".to_string()))
            }
        }

        let rb1 = rb.clone();
        // 错误路径场景:ok1(带 rollback)+ ok2(继续)+ failing → 逆序 rollback。
        struct Ok2Continue;
        #[async_trait]
        impl ChainCallback for Ok2Continue {
            fn name(&self) -> &'static str {
                "ok2-continue"
            }
            async fn call(&self, _ctx: &ChainContext) -> Result<ChainResult, RunnerError> {
                Ok(ChainResult::continue_(json!({"step": 2})))
            }
        }
        let mut guards = Vec::new();
        guards.push(chain.add(
            Arc::new(Ok1),
            5,
            true,
            0,
            None,
            Some(Arc::new(move |_ctx| {
                rb1.lock().unwrap().push("ok1-rollback".to_string());
            })),
        ));
        guards.push(chain.add(Arc::new(Ok2Continue), 4, true, 0, None, None));
        guards.push(chain.add(Arc::new(Failing), 3, true, 0, None, None));

        // 错误路径:ok1 成功 → ok2 成功 → failing 报错 → ok1 的 rollback 逆序执行。
        let error = chain.execute(vec![]).await.expect_err("failing");
        assert!(error.0.contains("failing"));
        assert_eq!(
            *rollbacks.lock().unwrap(),
            vec!["ok1-rollback"],
            "rollback on error"
        );

        // ROLLBACK 动作路径。
        rollbacks.lock().unwrap().clear();
        let chain2 = ctx.service::<dyn CallbackChain>(&RUNNER).expect("chain2");
        let rb2 = rollbacks.clone();
        let mut guards2 = Vec::new();
        guards2.push(chain2.add(
            Arc::new(Ok1),
            5,
            true,
            0,
            None,
            Some(Arc::new(move |_ctx| {
                rb2.lock().unwrap().push("ok1-rollback".to_string());
            })),
        ));
        guards2.push(chain2.add(Arc::new(Ok2), 4, true, 0, None, None));
        // 移除 failing,让 ok2 的 ROLLBACK 动作触发。
        chain2.remove("failing");
        let result = chain2.execute(vec![]).await.expect("rollback action");
        assert_eq!(result.action, ChainAction::Rollback);
        assert_eq!(
            *rollbacks.lock().unwrap(),
            vec!["ok1-rollback"],
            "rollback action"
        );

        drop(effects);
    }
}
