//! kv_cache seam:KV-cache 亲和策略钩子与 session 级动作语义。
//!
//! 对齐 `openjiuwen/harness/kv_cache/kv_cache_hooks.py` + `core/foundation/kv_cache/
//! kv_cache_session_actions.py` 的确定性部分:
//! - 策略钩子:`affinity_enabled` / `is_sticky_subagent_type` / `resolve_sub_session_id`;
//! - session 动作:`run_session_kv_action`(capability 检查关闭即失败、
//!   `{action}_kvc` 调用、timeout 解析)、`dispatch_session_kv_cache_signal`
//!   (offload/prefetch 信号调度,非法动作显式报错)、`evict_session_kv_cache`。
//!
//! 契约零实现:真实动作经 [`KvcAffinityModel`] trait 注入;后台调度语义(事件循环
//! 任务链)由插件提供。

use crate::seam::Seam;

/// session 级 KV-cache 动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKvcAction {
    Offload,
    Prefetch,
    Evict,
}

impl SessionKvcAction {
    /// 动作名(对齐 Python 字符串 `"offload"` / `"prefetch"` / `"evict"`)。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offload => "offload",
            Self::Prefetch => "prefetch",
            Self::Evict => "evict",
        }
    }
}

/// 可调度信号动作(仅 offload / prefetch;对齐 `SessionKVCacheSignalAction`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKvcSignal {
    Offload,
    Prefetch,
}

impl SessionKvcSignal {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offload => "offload",
            Self::Prefetch => "prefetch",
        }
    }
}

/// kv_cache 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvcError(pub String);

impl core::fmt::Display for KvcError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KvcError {}

/// 支持 KV-cache 亲和的模型(对齐 Python 模型的 duck-typed 面)。
pub trait KvcAffinityModel: Send + Sync {
    /// 是否支持 KV-cache 亲和(对齐 `supports_kv_cache_affinity()`)。
    fn supports_kv_cache_affinity(&self) -> bool;

    /// 执行一次 session 级动作(对齐 `{action}_kvc(target="session", ...)`)。
    fn action_kvc(
        &self,
        action: SessionKvcAction,
        session_id: &str,
        parent_session_id: &str,
        timeout_seconds: Option<f64>,
    ) -> Result<bool, KvcError>;
}

/// KV-cache 策略钩子(对齐 `harness/kv_cache/kv_cache_hooks.py`)。
pub trait KvcHooks: Seam {
    /// sticky subagent 类型判定(browser_agent / verification_agent)。
    fn is_sticky_subagent_type(&self, subagent_type: &str) -> bool;

    /// 解析子会话 id:metadata.sub_session_id 或 `{parent}_sub_{task_id|unknown}`。
    fn resolve_sub_session_id(
        &self,
        task_id: &str,
        parent_session_id: &str,
        metadata_sub_session_id: Option<&str>,
    ) -> String;

    /// prefetch sticky subagent 信号(affinity 关闭或非 sticky 直接返回)。
    fn prefetch_sticky_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        subagent_type: &str,
        sub_session_id: &str,
        parent_session_id: &str,
    );

    /// 完成子代理:成功 sticky → offload;否则 evict(affinity 关闭直接返回)。
    fn finish_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        subagent_type: &str,
        sub_session_id: &str,
        parent_session_id: &str,
        succeeded: bool,
    );

    /// 驱逐子代理(affinity 关闭直接返回)。
    fn evict_subagent(
        &self,
        model: Option<&dyn KvcAffinityModel>,
        affinity_enabled: bool,
        sub_session_id: &str,
        parent_session_id: &str,
    );
}

/// 解析 session 动作超时(对齐 `resolve_kvc_action_timeout(action, "session", timeout)`)。
///
/// 显式 timeout 优先;否则返回 None(由模型默认)。
pub fn resolve_kvc_action_timeout(
    _action: SessionKvcAction,
    explicit_timeout: Option<f64>,
) -> Option<f64> {
    explicit_timeout
}

/// 一次 session KV 动作的失败关闭语义(对齐 `_run_session_kv_action`)。
///
/// 返回 (succeeded, reason):false 且 reason 非 None 表示 capability/调用失败。
pub fn run_session_kv_action(
    model: Option<&dyn KvcAffinityModel>,
    action: SessionKvcAction,
    session_id: &str,
    parent_session_id: Option<&str>,
    timeout: Option<f64>,
    enabled: bool,
) -> Result<bool, KvcError> {
    if !enabled || model.is_none() || session_id.is_empty() {
        return Ok(false);
    }
    let model = model.expect("checked");
    if !model.supports_kv_cache_affinity() {
        return Ok(false);
    }
    let resolved_timeout = resolve_kvc_action_timeout(action, timeout);
    let parent = parent_session_id
        .filter(|p| !p.is_empty())
        .unwrap_or(session_id);
    model.action_kvc(action, session_id, parent, resolved_timeout)
}

/// 信号动作合法性(对齐 `dispatch_session_kv_cache_signal` 的 ValueError)。
pub fn validate_signal_action(action: SessionKvcSignal) -> Result<(), KvcError> {
    match action {
        SessionKvcSignal::Offload | SessionKvcSignal::Prefetch => Ok(()),
    }
}

/// sticky subagent 类型判定(对齐 `kv_cache_hooks.is_sticky_subagent_type`)。
pub fn is_sticky_subagent_type(subagent_type: &str) -> bool {
    matches!(subagent_type.trim(), "browser_agent" | "verification_agent")
}

/// 解析子会话 id(对齐 `kv_cache_hooks.resolve_sub_session_id`)。
pub fn resolve_sub_session_id(
    task_id: &str,
    parent_session_id: &str,
    metadata_sub_session_id: Option<&str>,
) -> String {
    if let Some(sub) = metadata_sub_session_id.filter(|s| !s.is_empty()) {
        return sub.to_string();
    }
    let safe_task_id = task_id.trim();
    let safe_task_id = if safe_task_id.is_empty() {
        "unknown"
    } else {
        safe_task_id
    };
    format!("{parent_session_id}_sub_{safe_task_id}")
}

// ---------------------------------------------------------------------------
// Team 级 KV-cache(对齐 agent_teams/kv_cache/kv_cache_lifecycle.py 确定性部分)
// ---------------------------------------------------------------------------

/// 团队成员 KV-cache 状态(对齐 `TeamKVCState`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamKvcState {
    Active,
    ReadyResident,
    Offloaded,
    Evicted,
}

impl TeamKvcState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ReadyResident => "ready_resident",
            Self::Offloaded => "offloaded",
            Self::Evicted => "evicted",
        }
    }
}

/// KV-cache 身份(对齐 `KVCacheIdentity`)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KvcCacheIdentity {
    pub cache_id: String,
    pub parent_cache_id: String,
}

/// 团队 KV 动作(对齐 `KVCAction = Literal["offload","prefetch","evict"]`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvcTeamAction {
    Offload,
    Prefetch,
    Evict,
}

impl KvcTeamAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offload => "offload",
            Self::Prefetch => "prefetch",
            Self::Evict => "evict",
        }
    }
}

/// 绑定是否可管理(对齐 `is_binding_manageable`)。
///
/// 要求:非空 + enabled + 模型支持亲和(capability 检查关闭即失败)+ cache_id
/// 与 parent_cache_id 均非空。
pub fn is_binding_manageable(
    enabled: bool,
    has_model: bool,
    model_supports_affinity: bool,
    cache_id: &str,
    parent_cache_id: &str,
) -> bool {
    enabled
        && has_model
        && model_supports_affinity
        && !cache_id.is_empty()
        && !parent_cache_id.is_empty()
}

/// 记录是否可执行某动作(对齐 `_record_actionable`)。
///
/// 未注册动作显式报错。
pub fn record_actionable(
    state: TeamKvcState,
    action: KvcTeamAction,
    manageable: bool,
) -> Result<bool, KvcError> {
    if !manageable {
        return Ok(false);
    }
    if state == TeamKvcState::Evicted {
        return Ok(false);
    }
    match action {
        KvcTeamAction::Offload => Ok(state != TeamKvcState::Offloaded),
        KvcTeamAction::Prefetch => Ok(state == TeamKvcState::Offloaded),
        KvcTeamAction::Evict => Ok(true),
    }
}

/// 绑定控制域(对齐 `binding_control_domain` 确定性部分)。
///
/// 缺失路由元数据时回退 `("model-object", id)` 域;否则
/// (provider, api_base, model_name, namespace, tenant, router)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlDomain {
    pub values: Vec<String>,
}

/// 归一化配置值(对齐 `_normalized_config_value`):取 `.value` 属性、strip。
pub fn normalized_config_value(value: Option<&str>) -> String {
    value.unwrap_or("").trim().to_string()
}

/// 构建控制域;`model_id` 用于缺失路由元数据时的对象域。
pub fn build_control_domain(
    provider: &str,
    api_base: &str,
    model_name: &str,
    namespace_values: &[Option<String>],
    model_id: &str,
) -> ControlDomain {
    let provider = normalized_config_value(Some(provider)).to_lowercase();
    let api_base = normalized_config_value(Some(api_base))
        .trim_end_matches('/')
        .to_string();
    let model_name = normalized_config_value(Some(model_name));
    if provider.is_empty() || api_base.is_empty() || model_name.is_empty() {
        return ControlDomain {
            values: vec!["model-object".to_string(), model_id.to_string()],
        };
    }
    let mut values = vec![provider, api_base, model_name];
    for ns in namespace_values {
        values.push(normalized_config_value(ns.as_deref()));
    }
    ControlDomain { values }
}

/// 动作后的目标状态(对齐 `_run_record_action` 的状态迁移)。
pub fn state_after_action(state: TeamKvcState, action: KvcTeamAction, ok: bool) -> TeamKvcState {
    match action {
        KvcTeamAction::Offload => {
            if ok {
                TeamKvcState::Offloaded
            } else {
                state
            }
        }
        KvcTeamAction::Prefetch => TeamKvcState::Active,
        KvcTeamAction::Evict => {
            if ok {
                TeamKvcState::Evicted
            } else {
                state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeModel {
        supports: bool,
        calls: AtomicUsize,
    }

    impl KvcAffinityModel for FakeModel {
        fn supports_kv_cache_affinity(&self) -> bool {
            self.supports
        }

        fn action_kvc(
            &self,
            _action: SessionKvcAction,
            _session_id: &str,
            parent_session_id: &str,
            _timeout_seconds: Option<f64>,
        ) -> Result<bool, KvcError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(parent_session_id, "parent-1");
            Ok(true)
        }
    }

    #[test]
    fn sticky_types_are_browser_and_verification() {
        assert!(is_sticky_subagent_type("browser_agent"));
        assert!(is_sticky_subagent_type("verification_agent"));
        assert!(!is_sticky_subagent_type("code_agent"));
        assert!(!is_sticky_subagent_type(""));
        assert!(!is_sticky_subagent_type("BROWSER_AGENT"));
    }

    #[test]
    fn resolve_sub_session_id_prefers_metadata() {
        assert_eq!(resolve_sub_session_id("t1", "p1", Some("meta-9")), "meta-9");
        assert_eq!(resolve_sub_session_id("t1", "p1", None), "p1_sub_t1");
        assert_eq!(resolve_sub_session_id("", "p1", None), "p1_sub_unknown");
        assert_eq!(resolve_sub_session_id("t1", "p1", Some("")), "p1_sub_t1");
    }

    #[test]
    fn run_action_requires_enabled_model_and_session() {
        let model = FakeModel {
            supports: true,
            calls: AtomicUsize::new(0),
        };
        assert!(
            !run_session_kv_action(Some(&model), SessionKvcAction::Evict, "", None, None, true)
                .expect("ok")
        );
        assert!(
            !run_session_kv_action(None, SessionKvcAction::Evict, "s1", None, None, true)
                .expect("ok")
        );
        assert!(
            !run_session_kv_action(
                Some(&model),
                SessionKvcAction::Evict,
                "s1",
                None,
                None,
                false
            )
            .expect("ok")
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn run_action_checks_capability_closed_failure() {
        let model = FakeModel {
            supports: false,
            calls: AtomicUsize::new(0),
        };
        assert!(
            !run_session_kv_action(
                Some(&model),
                SessionKvcAction::Offload,
                "s1",
                None,
                None,
                true
            )
            .expect("ok")
        );
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn run_action_invokes_model() {
        let model = FakeModel {
            supports: true,
            calls: AtomicUsize::new(0),
        };
        let ok = run_session_kv_action(
            Some(&model),
            SessionKvcAction::Prefetch,
            "s1",
            Some("parent-1"),
            Some(5.0),
            true,
        )
        .expect("ok");
        assert!(ok);
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn signal_action_validation() {
        assert!(validate_signal_action(SessionKvcSignal::Offload).is_ok());
        assert!(validate_signal_action(SessionKvcSignal::Prefetch).is_ok());
    }

    #[test]
    fn team_state_enum_strings() {
        assert_eq!(TeamKvcState::Active.as_str(), "active");
        assert_eq!(TeamKvcState::ReadyResident.as_str(), "ready_resident");
        assert_eq!(TeamKvcState::Offloaded.as_str(), "offloaded");
        assert_eq!(TeamKvcState::Evicted.as_str(), "evicted");
    }

    #[test]
    fn binding_manageable_requires_all() {
        // enabled + model + supports + cache ids。
        assert!(is_binding_manageable(true, true, true, "c", "p"));
        assert!(!is_binding_manageable(false, true, true, "c", "p"));
        assert!(!is_binding_manageable(true, false, true, "c", "p"));
        assert!(!is_binding_manageable(true, true, false, "c", "p"));
        assert!(!is_binding_manageable(true, true, true, "", "p"));
        assert!(!is_binding_manageable(true, true, true, "c", ""));
    }

    #[test]
    fn record_actionable_rules() {
        use KvcTeamAction::*;
        // offload:非 OFFLOADED 可执行。
        assert!(record_actionable(TeamKvcState::Active, Offload, true).expect("ok"));
        assert!(!record_actionable(TeamKvcState::Offloaded, Offload, true).expect("ok"));
        // prefetch:仅 OFFLOADED。
        assert!(record_actionable(TeamKvcState::Offloaded, Prefetch, true).expect("ok"));
        assert!(!record_actionable(TeamKvcState::Active, Prefetch, true).expect("ok"));
        // evict:任意非 EVICTED。
        assert!(record_actionable(TeamKvcState::ReadyResident, Evict, true).expect("ok"));
        // EVICTED:全部不可执行。
        assert!(!record_actionable(TeamKvcState::Evicted, Evict, true).expect("ok"));
        // 不可管理:false。
        assert!(!record_actionable(TeamKvcState::Active, Evict, false).expect("ok"));
    }

    #[test]
    fn control_domain_grouping() {
        // 完整路由元数据 → (provider, api_base, model_name, namespaces)。
        let domain = build_control_domain(
            "OpenAI",
            "https://api.openai.com/",
            "gpt-4",
            &[Some("ns1".to_string()), None, Some("tenant-9".to_string())],
            "obj-1",
        );
        assert_eq!(
            domain.values,
            vec![
                "openai",
                "https://api.openai.com",
                "gpt-4",
                "ns1",
                "",
                "tenant-9"
            ]
        );
        // 缺失路由元数据 → model-object 域。
        let fallback = build_control_domain("", "https://x", "", &[], "obj-42");
        assert_eq!(fallback.values, vec!["model-object", "obj-42"]);
    }

    #[test]
    fn state_after_action_transitions() {
        use KvcTeamAction::*;
        // offload 成功 → OFFLOADED;失败保持。
        assert_eq!(
            state_after_action(TeamKvcState::Active, Offload, true),
            TeamKvcState::Offloaded
        );
        assert_eq!(
            state_after_action(TeamKvcState::Active, Offload, false),
            TeamKvcState::Active
        );
        // prefetch 无论成败 → ACTIVE。
        assert_eq!(
            state_after_action(TeamKvcState::Offloaded, Prefetch, true),
            TeamKvcState::Active
        );
        assert_eq!(
            state_after_action(TeamKvcState::Offloaded, Prefetch, false),
            TeamKvcState::Active
        );
        // evict 成功 → EVICTED。
        assert_eq!(
            state_after_action(TeamKvcState::ReadyResident, Evict, true),
            TeamKvcState::Evicted
        );
    }
}
