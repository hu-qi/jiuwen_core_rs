//! # ah-plugins-kv-cache
//!
//! 真实 KV-cache 策略钩子(1:1 对齐 `openjiuwen/harness/kv_cache/kv_cache_hooks.py`
//! 确定性部分):
//! - `prefetch_sticky_subagent`:affinity 开启 + sticky 类型 → 调度 prefetch 信号;
//! - `finish_subagent`:成功 sticky → offload;否则 evict;
//! - `evict_subagent`:evict;
//! - `is_sticky_subagent_type` / `resolve_sub_session_id` 委托契约层纯函数。
//!
//! 动作执行经 [`KvcAffinityModel`] seam 注入,失败显式记录(无静默 fallback)。

use std::sync::Arc;

use ah_contracts::keys::KVC_HOOKS;
use ah_contracts::kv_cache::{
    KvcAffinityModel, KvcHooks, SessionKvcAction, is_sticky_subagent_type, resolve_sub_session_id,
    run_session_kv_action,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实 KV-cache 钩子实现。
pub struct KvcHooksImpl;

impl Seam for KvcHooksImpl {}

impl KvcHooks for KvcHooksImpl {
    fn is_sticky_subagent_type(&self, subagent_type: &str) -> bool {
        is_sticky_subagent_type(subagent_type)
    }

    fn resolve_sub_session_id(
        &self,
        task_id: &str,
        parent_session_id: &str,
        metadata_sub_session_id: Option<&str>,
    ) -> String {
        resolve_sub_session_id(task_id, parent_session_id, metadata_sub_session_id)
    }

    fn prefetch_sticky_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        subagent_type: &str,
        sub_session_id: &str,
        parent_session_id: &str,
    ) {
        if !affinity_enabled || !is_sticky_subagent_type(subagent_type) {
            return;
        }
        let _ = run_session_kv_action(
            model,
            SessionKvcAction::Prefetch,
            sub_session_id,
            Some(parent_session_id),
            None,
            true,
        );
    }

    fn finish_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        subagent_type: &str,
        sub_session_id: &str,
        parent_session_id: &str,
        succeeded: bool,
    ) {
        if !affinity_enabled {
            return;
        }
        if succeeded && is_sticky_subagent_type(subagent_type) {
            let _ = run_session_kv_action(
                model,
                SessionKvcAction::Offload,
                sub_session_id,
                Some(parent_session_id),
                None,
                true,
            );
            return;
        }
        let _ = run_session_kv_action(
            model,
            SessionKvcAction::Evict,
            sub_session_id,
            Some(parent_session_id),
            None,
            true,
        );
    }

    fn evict_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        sub_session_id: &str,
        parent_session_id: &str,
    ) {
        if !affinity_enabled {
            return;
        }
        let _ = run_session_kv_action(
            model,
            SessionKvcAction::Evict,
            sub_session_id,
            Some(parent_session_id),
            None,
            true,
        );
    }
}

/// kv-cache 插件:注册 `kvc-hooks` seam。
pub struct KvcCachePlugin;

impl Plugin for KvcCachePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-kv-cache"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![KVC_HOOKS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let hooks: Arc<dyn KvcHooks> = Arc::new(KvcHooksImpl);
        Ok(vec![ctx.register(KVC_HOOKS, hooks)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::KVC_HOOKS;
    use ah_contracts::kv_cache::KvcError;
    use ah_hub::plugin::DynPlugin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 记录被调用动作的测试模型。
    struct RecordingModel {
        supports: bool,
        counter: AtomicUsize,
    }

    impl RecordingModel {
        fn new(supports: bool) -> Self {
            Self {
                supports,
                counter: AtomicUsize::new(0),
            }
        }
    }

    impl KvcAffinityModel for RecordingModel {
        fn supports_kv_cache_affinity(&self) -> bool {
            self.supports
        }

        fn action_kvc(
            &self,
            action: SessionKvcAction,
            _session_id: &str,
            _parent_session_id: &str,
            _timeout_seconds: Option<f64>,
        ) -> Result<bool, KvcError> {
            self.counter.fetch_add(1, Ordering::SeqCst);
            let _ = action;
            Ok(true)
        }
    }

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(KvcCachePlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn hooks_seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn KvcHooks>(&KVC_HOOKS).is_some());
        drop(effects);
    }

    #[test]
    fn sticky_and_sub_session_resolution() {
        let (ctx, effects) = build_ctx();
        let hooks = ctx.service::<dyn KvcHooks>(&KVC_HOOKS).expect("hooks");
        assert!(hooks.is_sticky_subagent_type("browser_agent"));
        assert!(!hooks.is_sticky_subagent_type("code_agent"));
        assert_eq!(hooks.resolve_sub_session_id("t9", "p1", None), "p1_sub_t9");
        assert_eq!(hooks.resolve_sub_session_id("t9", "p1", Some("m1")), "m1");
        drop(effects);
    }

    #[test]
    fn prefetch_skipped_when_disabled_or_non_sticky() {
        let (ctx, effects) = build_ctx();
        let hooks = ctx.service::<dyn KvcHooks>(&KVC_HOOKS).expect("hooks");
        let model = RecordingModel::new(true);
        // affinity 关闭 → 不调度。
        hooks.prefetch_sticky_subagent(Some(&model), false, "browser_agent", "sub", "parent");
        assert_eq!(model.counter.load(Ordering::SeqCst), 0);
        // 非 sticky → 不调度。
        hooks.prefetch_sticky_subagent(Some(&model), true, "code_agent", "sub", "parent");
        assert_eq!(model.counter.load(Ordering::SeqCst), 0);
        drop(effects);
    }

    #[test]
    fn finish_evicts_failed_or_non_sticky() {
        let (ctx, effects) = build_ctx();
        let hooks = ctx.service::<dyn KvcHooks>(&KVC_HOOKS).expect("hooks");
        let model = RecordingModel::new(true);
        // 失败 → evict。
        hooks.finish_subagent(Some(&model), true, "browser_agent", "sub", "parent", false);
        // 非 sticky 成功 → evict。
        hooks.finish_subagent(Some(&model), true, "code_agent", "sub", "parent", true);
        assert_eq!(model.counter.load(Ordering::SeqCst), 2);
        drop(effects);
    }

    #[test]
    fn finish_offloads_successful_sticky() {
        let (ctx, effects) = build_ctx();
        let hooks = ctx.service::<dyn KvcHooks>(&KVC_HOOKS).expect("hooks");
        let model = RecordingModel::new(true);
        hooks.finish_subagent(
            Some(&model),
            true,
            "verification_agent",
            "sub",
            "parent",
            true,
        );
        assert_eq!(model.counter.load(Ordering::SeqCst), 1);
        drop(effects);
    }

    #[test]
    fn evict_skipped_when_disabled() {
        let (ctx, effects) = build_ctx();
        let hooks = ctx.service::<dyn KvcHooks>(&KVC_HOOKS).expect("hooks");
        let model = RecordingModel::new(true);
        hooks.evict_subagent(Some(&model), false, "sub", "parent");
        assert_eq!(model.counter.load(Ordering::SeqCst), 0);
        hooks.evict_subagent(Some(&model), true, "sub", "parent");
        assert_eq!(model.counter.load(Ordering::SeqCst), 1);
        drop(effects);
    }
}
