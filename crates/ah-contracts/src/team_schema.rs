//! team-schema seam:agent_teams 装配蓝图与任务图规格(对齐
//! `agent_teams/schema/{ssh_transport,task,blueprint}.py` 的确定性部分)。
//!
//! - ssh_transport:`SshTransportConfig`(主机/端口/认证方式/超时)+
//!   `validate_ssh_auth`(key_file/password/agent 至少其一);
//! - task:纯数据模型 `TaskOpResult` / `TaskCreateResult` / `TaskSummary` /
//!   `TaskDetail` / `TaskListResult` / `TaskGraphSpec` / `TaskGraphResult` /
//!   `NewTaskSpec` / `GraphMutationResult`(任务图批量操作的结果与规格);
//! - blueprint:TransportSpec / StorageSpec 注册表(register→Effect 可逆)+
//!   `transport_merged_params` / `storage_merged_params`(backend/db_type 注入)、
//!   保留成员名 / 默认 leader 名常量、`validate_pool_router_exclusive` /
//!   `validate_external_cli_unique` / `validate_review_settings` /
//!   `validate_stall_settings` / `validate_reserved_names` /
//!   `validate_hitt_consistency` / `validate_bridge_consistency` /
//!   `validate_swarmflow_budget`(装配期确定性校验)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现(注册表经 Effect 可逆,由插件提供存储)。

use crate::effect::Effect;
use crate::seam::Seam;
use serde_json::Value;
use std::collections::BTreeMap;

/// 用户伪成员名(团队外部真人;对齐 USER_PSEUDO_MEMBER_NAME)。
pub const USER_PSEUDO_MEMBER_NAME: &str = "user";
/// 默认 leader 成员名(对齐 DEFAULT_LEADER_MEMBER_NAME)。
pub const DEFAULT_LEADER_MEMBER_NAME: &str = "team_leader";
/// 保留成员名(用户声明成员禁止使用;对齐 RESERVED_MEMBER_NAMES)。
pub const RESERVED_MEMBER_NAMES: &[&str] = &[
    "human_agent",
    USER_PSEUDO_MEMBER_NAME,
    DEFAULT_LEADER_MEMBER_NAME,
];

/// 装配蓝图错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError(pub String);

impl core::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SchemaError {}

// ---------------------------------------------------------------------------
// ssh_transport
// ---------------------------------------------------------------------------

/// 一个可达 ssh 端点的连接详情(对齐 SshTransportConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SshTransportConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub key_file: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub agent: bool,
    #[serde(default)]
    pub known_hosts: Option<String>,
    #[serde(default)]
    pub disable_host_key_check: bool,
    #[serde(default = "default_ssh_timeout")]
    pub connect_timeout_s: f64,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_timeout() -> f64 {
    15.0
}

/// 校验至少配置一种认证方式(对齐 `_require_auth`)。
///
/// key_file / password / agent 三者至少其一,否则显式 Err
/// (对齐 `AGENT_TEAM_CONFIG_INVALID` 语义)。
pub fn validate_ssh_auth(config: &SshTransportConfig) -> Result<(), SchemaError> {
    if config.key_file.is_none() && config.password.is_none() && !config.agent {
        Err(SchemaError(
            "SshTransportConfig requires at least one auth method (key_file / password / agent=True)"
                .to_string(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// task models
// ---------------------------------------------------------------------------

/// 任务变更结果(对齐 TaskOpResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskOpResult {
    pub ok: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl TaskOpResult {
    pub fn success(data: Option<Value>) -> Self {
        Self {
            ok: true,
            reason: String::new(),
            data,
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: reason.into(),
            data: None,
        }
    }
}

/// 任务创建结果(对齐 TaskCreateResult;`ok` = task 非 None)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskCreateResult {
    #[serde(default)]
    pub task: Option<Value>,
    #[serde(default)]
    pub reason: String,
}

impl TaskCreateResult {
    pub fn ok(&self) -> bool {
        self.task.is_some()
    }

    pub fn success(task: Value) -> Self {
        Self {
            task: Some(task),
            reason: String::new(),
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            task: None,
            reason: reason.into(),
        }
    }
}

/// 任务摘要(list/claimable 动作返回;对齐 TaskSummary)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

/// 任务详情(get 动作返回;对齐 TaskDetail)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskDetail {
    pub task_id: String,
    pub title: String,
    pub content: String,
    pub status: String,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub reviewer: Vec<String>,
    #[serde(default)]
    pub review_round: u64,
    #[serde(default)]
    pub max_review_rounds: Option<u64>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
}

/// list/claimable 动作响应(对齐 TaskListResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskListResult {
    pub tasks: Vec<TaskSummary>,
    pub count: usize,
}

/// 任务图批量添加中的一条任务(对齐 TaskGraphSpec)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskGraphSpec {
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub depended_by: Vec<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub reviewer: Vec<String>,
    #[serde(default)]
    pub max_review_rounds: Option<u64>,
}

/// 任务图批量添加结果(对齐 TaskGraphResult;批量原子性)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskGraphResult {
    pub ok: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub tasks: Vec<Value>,
}

impl TaskGraphResult {
    pub fn success(tasks: Vec<Value>) -> Self {
        Self {
            ok: true,
            reason: String::new(),
            tasks,
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: reason.into(),
            tasks: Vec::new(),
        }
    }
}

/// 依赖图变更中的一条新任务(对齐 NewTaskSpec)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewTaskSpec {
    pub task_id: String,
    pub title: String,
    pub content: String,
    pub initial_status: String,
    #[serde(default)]
    pub assignee: Option<String>,
    /// JSON 编码的 reviewer 成员名列表(或 None),逐字写入 reviewer 列。
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub max_review_rounds: Option<u64>,
}

/// 依赖图变更结果(对齐 GraphMutationResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphMutationResult {
    pub ok: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub refreshed_tasks: Vec<Value>,
}

impl GraphMutationResult {
    pub fn success(refreshed_tasks: Option<Vec<Value>>) -> Self {
        Self {
            ok: true,
            reason: String::new(),
            refreshed_tasks: refreshed_tasks.unwrap_or_default(),
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            reason: reason.into(),
            refreshed_tasks: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// blueprint: transport / storage 注册表
// ---------------------------------------------------------------------------

/// 注册表错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryError(pub String);

impl core::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistryError {}

/// transport / storage 注册表 Seam(Service Definition)。
///
/// 对齐 `register_transport` / `register_storage` / `TransportSpec.build` /
/// `StorageSpec.build`:按类型名注册配置类,`build` 注入 backend/db_type 键
/// 并校验类型存在(未知类型显式 Err,不静默)。注册返回 Effect(drop 即回滚)。
pub trait InfraRegistry: Seam {
    /// 注册一个 transport 配置类型;重复注册覆盖(对齐 dict 覆盖语义)。
    fn register_transport(&self, name: &str) -> Effect;

    /// 注册一个 storage 配置类型;重复注册覆盖。
    fn register_storage(&self, name: &str) -> Effect;

    /// 惰性注册内置类型(对齐 `_ensure_builtin_infra_registered`):
    /// transport = inprocess / pyzmq / hybrid;storage = sqlite /
    /// postgresql / mysql / memory。
    fn ensure_builtin(&self) -> Vec<Effect>;

    /// 解析 transport 配置(未知类型显式 Err;合并 `{"backend": type}`)。
    fn resolve_transport(
        &self,
        r#type: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<Value, RegistryError>;

    /// 解析 storage 配置(未知类型显式 Err;合并 `{"db_type": type}`)。
    fn resolve_storage(
        &self,
        r#type: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<Value, RegistryError>;
}

/// 注入 transport backend 键(对齐 `{"backend": self.type, **params}`)。
pub fn transport_merged_params(r#type: &str, params: &BTreeMap<String, Value>) -> Value {
    let mut merged = params.clone();
    merged.insert("backend".to_string(), Value::String(r#type.to_string()));
    serde_json::to_value(merged).unwrap_or(Value::Null)
}

/// 注入 storage db_type 键(对齐 `{"db_type": self.type, **params}`)。
pub fn storage_merged_params(r#type: &str, params: &BTreeMap<String, Value>) -> Value {
    let mut merged = params.clone();
    merged.insert("db_type".to_string(), Value::String(r#type.to_string()));
    serde_json::to_value(merged).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// blueprint: TeamAgentSpec 装配期确定性校验
// ---------------------------------------------------------------------------

/// model_pool / model_router / model_intelli_router 互斥校验
/// (对齐 `_validate_pool_router_exclusive`)。
pub fn validate_pool_router_exclusive(
    has_pool: bool,
    has_router: bool,
    has_intelli_router: bool,
) -> Result<(), SchemaError> {
    let mut configured = Vec::new();
    if has_pool {
        configured.push("model_pool");
    }
    if has_router {
        configured.push("model_router");
    }
    if has_intelli_router {
        configured.push("model_intelli_router");
    }
    if configured.len() > 1 {
        Err(SchemaError(format!(
            "model_pool, model_router and model_intelli_router are mutually exclusive; \
             configure exactly one (got: {configured:?})"
        )))
    } else {
        Ok(())
    }
}

/// external_cli_agents 的 cli_agent 名去重校验(对齐 `_validate_external_cli_unique`)。
pub fn validate_external_cli_unique(cli_agents: &[String]) -> Result<(), SchemaError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut duplicates = std::collections::BTreeSet::new();
    for name in cli_agents {
        if !seen.insert(name.clone()) {
            duplicates.insert(name.clone());
        }
    }
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(SchemaError(format!(
            "external_cli_agents has duplicate cli_agent name(s) {duplicates:?}; \
             declare each CLI kind at most once"
        )))
    }
}

/// verify-gate 旋钮范围校验(对齐 `_validate_review_settings`)。
pub fn validate_review_settings(
    verify_vote_threshold: f64,
    default_max_review_rounds: u64,
    review_stall_timeout: f64,
) -> Result<(), SchemaError> {
    if !(0.0 < verify_vote_threshold && verify_vote_threshold <= 1.0) {
        return Err(SchemaError(format!(
            "verify_vote_threshold must be in (0, 1], got {verify_vote_threshold}"
        )));
    }
    if default_max_review_rounds < 1 {
        return Err(SchemaError(format!(
            "default_max_review_rounds must be >= 1, got {default_max_review_rounds}"
        )));
    }
    if review_stall_timeout <= 0.0 {
        return Err(SchemaError(format!(
            "review_stall_timeout must be > 0 seconds, got {review_stall_timeout}"
        )));
    }
    Ok(())
}

/// autonomous 停滞提醒阈值校验(对齐 `_validate_stall_settings`)。
pub fn validate_stall_settings(
    stale_claim_idle_timeout: f64,
    stale_pending_idle_timeout: f64,
) -> Result<(), SchemaError> {
    if stale_claim_idle_timeout <= 0.0 {
        return Err(SchemaError(format!(
            "stale_claim_idle_timeout must be > 0 seconds, got {stale_claim_idle_timeout}"
        )));
    }
    if stale_pending_idle_timeout <= 0.0 {
        return Err(SchemaError(format!(
            "stale_pending_idle_timeout must be > 0 seconds, got {stale_pending_idle_timeout}"
        )));
    }
    Ok(())
}

/// swarmflow 预算校验(对齐 `_validate_swarmflow_budget`;None 或 >=1 通过)。
pub fn validate_swarmflow_budget(swarmflow_budget: Option<u64>) -> Result<(), SchemaError> {
    match swarmflow_budget {
        None => Ok(()),
        Some(budget) if budget >= 1 => Ok(()),
        Some(budget) => Err(SchemaError(format!(
            "swarmflow_budget must be >= 1 when set (got {budget})"
        ))),
    }
}

/// 保留成员名校验(对齐 `_validate_reserved_names`)。
///
/// leader 允许默认 `team_leader`,但不可占用 user/human_agent;预定义成员
/// (除 HUMAN_AGENT 角色)不可使用任何保留名。
pub fn validate_reserved_names(
    leader_member_name: &str,
    predefined: &[(String, String)],
) -> Result<(), SchemaError> {
    // leader_forbidden = RESERVED - {team_leader}
    let leader_forbidden: Vec<&str> = RESERVED_MEMBER_NAMES
        .iter()
        .copied()
        .filter(|n| *n != DEFAULT_LEADER_MEMBER_NAME)
        .collect();
    if leader_forbidden.contains(&leader_member_name) {
        return Err(SchemaError(format!(
            "LeaderSpec.member_name '{leader_member_name}' is reserved; pick a different name"
        )));
    }
    for (role_type, member_name) in predefined {
        if role_type == "human_agent" {
            continue;
        }
        if RESERVED_MEMBER_NAMES.contains(&member_name.as_str()) {
            return Err(SchemaError(format!(
                "predefined member '{member_name}' uses a reserved name \
                 (reserved: {RESERVED_MEMBER_NAMES:?})"
            )));
        }
    }
    Ok(())
}

/// HITT 一致性校验(对齐 `_validate_hitt_consistency`)。
///
/// 声明 HUMAN_AGENT 预定义成员而 enable_hitt=False → Err。
pub fn validate_hitt_consistency(
    enable_hitt: bool,
    predefined: &[(String, String)],
) -> Result<(), SchemaError> {
    if enable_hitt {
        return Ok(());
    }
    let offenders: Vec<&str> = predefined
        .iter()
        .filter(|(role_type, _)| role_type == "human_agent")
        .map(|(_, name)| name.as_str())
        .collect();
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(SchemaError(format!(
            "predefined_members contains HUMAN_AGENT role(s) {offenders:?} but \
             enable_hitt=False; set enable_hitt=True (capability ceiling) or \
             remove the human member(s)"
        )))
    }
}

/// bridge 一致性校验(对齐 `_validate_bridge_consistency`)。
///
/// 声明 BRIDGE_AGENT 预定义成员而 enable_bridge=False → Err。
pub fn validate_bridge_consistency(
    enable_bridge: bool,
    predefined: &[(String, String)],
) -> Result<(), SchemaError> {
    if enable_bridge {
        return Ok(());
    }
    let offenders: Vec<&str> = predefined
        .iter()
        .filter(|(role_type, _)| role_type == "bridge_agent")
        .map(|(_, name)| name.as_str())
        .collect();
    if offenders.is_empty() {
        Ok(())
    } else {
        Err(SchemaError(format!(
            "predefined_members contains BRIDGE_AGENT role(s) {offenders:?} but \
             enable_bridge=False; set enable_bridge=True (capability ceiling) or \
             remove the bridge member(s)"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ssh_auth_validation() {
        let mut cfg = SshTransportConfig {
            host: "10.0.0.1".to_string(),
            port: 22,
            username: None,
            key_file: None,
            password: None,
            agent: false,
            known_hosts: None,
            disable_host_key_check: false,
            connect_timeout_s: 15.0,
        };
        assert!(validate_ssh_auth(&cfg).is_err(), "无认证方式 → Err");
        let err = validate_ssh_auth(&cfg).unwrap_err();
        assert!(err.0.contains("at least one auth method"), "{err}");
        // key_file / password / agent 任一即可。
        cfg.key_file = Some("/home/u/.ssh/id_rsa".to_string());
        assert!(validate_ssh_auth(&cfg).is_ok());
        cfg.key_file = None;
        cfg.password = Some("secret".to_string());
        assert!(validate_ssh_auth(&cfg).is_ok());
        cfg.password = None;
        cfg.agent = true;
        assert!(validate_ssh_auth(&cfg).is_ok());
    }

    #[test]
    fn ssh_config_serde_defaults() {
        let cfg: SshTransportConfig =
            serde_json::from_value(json!({"host": "h"})).expect("host only");
        assert_eq!(cfg.port, 22);
        assert_eq!(cfg.connect_timeout_s, 15.0);
        assert!(!cfg.agent);
    }

    #[test]
    fn task_op_result_success_fail() {
        let ok = TaskOpResult::success(Some(json!({"tally": 2})));
        assert!(ok.ok);
        assert!(ok.data.is_some());
        let fail = TaskOpResult::fail("cycle detected");
        assert!(!fail.ok);
        assert_eq!(fail.reason, "cycle detected");
    }

    #[test]
    fn task_create_result_ok_delegates() {
        assert!(TaskCreateResult::success(json!({"task_id": "t1"})).ok());
        let fail = TaskCreateResult::fail("bad title");
        assert!(!fail.ok());
        assert!(fail.task.is_none());
    }

    #[test]
    fn task_graph_and_mutation_results() {
        let g = TaskGraphResult::success(vec![json!({"task_id": "t1"})]);
        assert!(g.ok);
        assert_eq!(g.tasks.len(), 1);
        let gf = TaskGraphResult::fail("cycle");
        assert!(!gf.ok);
        assert!(gf.tasks.is_empty());

        let m = GraphMutationResult::success(Some(vec![json!({"task_id": "t1"})]));
        assert!(m.ok);
        assert_eq!(m.refreshed_tasks.len(), 1);
        let mf = GraphMutationResult::fail("terminal target");
        assert!(!mf.ok);
    }

    #[test]
    fn merged_params_inject_kind() {
        let params = BTreeMap::from([("timeout".to_string(), json!(5.0))]);
        let t = transport_merged_params("inprocess", &params);
        assert_eq!(t["backend"], "inprocess");
        assert_eq!(t["timeout"], 5.0);
        let s = storage_merged_params("sqlite", &BTreeMap::new());
        assert_eq!(s["db_type"], "sqlite");
    }

    #[test]
    fn pool_router_exclusive() {
        assert!(validate_pool_router_exclusive(false, false, false).is_ok());
        assert!(validate_pool_router_exclusive(true, false, false).is_ok());
        let err = validate_pool_router_exclusive(true, true, false).unwrap_err();
        assert!(err.0.contains("mutually exclusive"), "{err}");
        let err2 = validate_pool_router_exclusive(true, false, true).unwrap_err();
        assert!(
            err2.0
                .contains("got: [\"model_pool\", \"model_intelli_router\"]"),
            "{err2}"
        );
    }

    #[test]
    fn external_cli_unique() {
        assert!(validate_external_cli_unique(&["claude".to_string(), "codex".to_string()]).is_ok());
        let err = validate_external_cli_unique(&["claude".to_string(), "claude".to_string()])
            .unwrap_err();
        assert!(err.0.contains("duplicate cli_agent"), "{err}");
    }

    #[test]
    fn review_and_stall_settings() {
        assert!(validate_review_settings(0.5, 3, 60.0).is_ok());
        assert!(validate_review_settings(0.0, 3, 60.0).is_err());
        assert!(validate_review_settings(1.1, 3, 60.0).is_err());
        assert!(validate_review_settings(0.5, 0, 60.0).is_err());
        assert!(validate_review_settings(0.5, 3, 0.0).is_err());
        assert!(validate_stall_settings(30.0, 60.0).is_ok());
        assert!(validate_stall_settings(0.0, 60.0).is_err());
        assert!(validate_stall_settings(30.0, -1.0).is_err());
    }

    #[test]
    fn swarmflow_budget() {
        assert!(validate_swarmflow_budget(None).is_ok());
        assert!(validate_swarmflow_budget(Some(1)).is_ok());
        assert!(validate_swarmflow_budget(Some(0)).is_err());
    }

    #[test]
    fn reserved_names() {
        // leader 默认名允许。
        assert!(validate_reserved_names("team_leader", &[]).is_ok());
        // leader 占用 user → Err。
        let err = validate_reserved_names("user", &[]).unwrap_err();
        assert!(err.0.contains("reserved"), "{err}");
        // teammate 占用 human_agent → Err;HUMAN_AGENT 角色豁免。
        let err2 = validate_reserved_names(
            "team_leader",
            &[("teammate".to_string(), "human_agent".to_string())],
        )
        .unwrap_err();
        assert!(err2.0.contains("reserved name"), "{err2}");
        assert!(
            validate_reserved_names(
                "team_leader",
                &[("human_agent".to_string(), "human_agent".to_string())]
            )
            .is_ok()
        );
    }

    #[test]
    fn hitt_and_bridge_consistency() {
        // enable_hitt=True → 允许预定义 human_agent。
        assert!(
            validate_hitt_consistency(true, &[("human_agent".to_string(), "ha".to_string())])
                .is_ok()
        );
        // enable_hitt=False + 预定义 human_agent → Err。
        let err =
            validate_hitt_consistency(false, &[("human_agent".to_string(), "ha".to_string())])
                .unwrap_err();
        assert!(err.0.contains("enable_hitt=False"), "{err}");
        assert!(validate_hitt_consistency(false, &[]).is_ok());

        assert!(
            validate_bridge_consistency(true, &[("bridge_agent".to_string(), "ba".to_string())])
                .is_ok()
        );
        let err2 =
            validate_bridge_consistency(false, &[("bridge_agent".to_string(), "ba".to_string())])
                .unwrap_err();
        assert!(err2.0.contains("enable_bridge=False"), "{err2}");
        assert!(validate_bridge_consistency(false, &[]).is_ok());
    }
}
