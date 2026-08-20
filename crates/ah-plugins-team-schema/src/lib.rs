//! # ah-plugins-team-schema
//!
//! 真实 agent_teams schema 装配(1:1 对齐
//! `agent_teams/schema/{ssh_transport,task,blueprint}.py` 的确定性部分):
//! - 纯函数委托:ssh 认证校验、任务图模型结果构造、blueprint 装配期校验
//!   (pool/router 互斥、cli_agent 去重、review/stall/预算范围、保留名、
//!   HITT/bridge 一致性)——全部委托契约层;
//! - `InfraRegistry`:transport/storage 注册表(register → Effect 可逆;
//!   ensure_builtin 惰性注册 inprocess/pyzmq/hybrid + sqlite/postgresql/mysql/
//!   memory;resolve 注入 backend/db_type 键,未知类型显式 Err)。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use ah_contracts::effect::Effect;
use ah_contracts::keys::{INFRA_REGISTRY, TEAM_SCHEMA};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::team_schema::{
    InfraRegistry, RegistryError, SchemaError, SshTransportConfig, TaskCreateResult,
    TaskGraphResult, TaskOpResult, validate_ssh_auth,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 内置 transport 类型(对齐 `_ensure_builtin_infra_registered`)。
const BUILTIN_TRANSPORTS: &[&str] = &["inprocess", "pyzmq", "hybrid"];
/// 内置 storage 类型。
const BUILTIN_STORAGES: &[&str] = &["sqlite", "postgresql", "mysql", "memory"];

/// 真实 transport/storage 注册表。
///
/// 注册返回可逆 Effect:drop 时移除该类型(dict 覆盖语义:重复注册覆盖,
/// 回滚恢复前值)。`ensure_builtin` 只在注册表为空时注册内置类型
/// (对齐 Python `if not _TRANSPORT_REGISTRY` 的惰性初始化)。
pub struct InfraRegistryImpl {
    inner: Arc<RegistryState>,
}

struct RegistryState {
    transports: Mutex<HashMap<String, ()>>,
    storages: Mutex<HashMap<String, ()>>,
}

impl Default for InfraRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl InfraRegistryImpl {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RegistryState {
                transports: Mutex::new(HashMap::new()),
                storages: Mutex::new(HashMap::new()),
            }),
        }
    }
}

impl Seam for InfraRegistryImpl {}

impl InfraRegistry for InfraRegistryImpl {
    fn register_transport(&self, name: &str) -> Effect {
        let mut map = self.inner.transports.lock().unwrap();
        let previous = map.insert(name.to_string(), ());
        let inner = self.inner.clone();
        let name = name.to_string();
        // Effect:drop 时恢复前值(覆盖语义可逆)。
        Effect::new(move || {
            let mut map = inner.transports.lock().unwrap();
            match previous {
                Some(_) => {
                    map.insert(name.clone(), ());
                }
                None => {
                    map.remove(&name);
                }
            }
        })
    }

    fn register_storage(&self, name: &str) -> Effect {
        let mut map = self.inner.storages.lock().unwrap();
        let previous = map.insert(name.to_string(), ());
        let inner = self.inner.clone();
        let name = name.to_string();
        Effect::new(move || {
            let mut map = inner.storages.lock().unwrap();
            match previous {
                Some(_) => {
                    map.insert(name.clone(), ());
                }
                None => {
                    map.remove(&name);
                }
            }
        })
    }

    fn ensure_builtin(&self) -> Vec<Effect> {
        let mut t = self.inner.transports.lock().unwrap();
        if t.is_empty() {
            for name in BUILTIN_TRANSPORTS {
                t.insert((*name).to_string(), ());
            }
        }
        drop(t);
        let mut s = self.inner.storages.lock().unwrap();
        if s.is_empty() {
            for name in BUILTIN_STORAGES {
                s.insert((*name).to_string(), ());
            }
        }
        // 内置注册是启动基线,无独立回滚 guard。
        Vec::new()
    }

    fn resolve_transport(
        &self,
        r#type: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<Value, RegistryError> {
        let map = self.inner.transports.lock().unwrap();
        if !map.contains_key(r#type) {
            return Err(RegistryError(format!(
                "Unknown transport type '{type}'. Registered types: {:?}",
                map.keys().collect::<Vec<_>>()
            )));
        }
        Ok(ah_contracts::team_schema::transport_merged_params(
            r#type, params,
        ))
    }

    fn resolve_storage(
        &self,
        r#type: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<Value, RegistryError> {
        let map = self.inner.storages.lock().unwrap();
        if !map.contains_key(r#type) {
            return Err(RegistryError(format!(
                "Unknown storage type '{type}'. Registered types: {:?}",
                map.keys().collect::<Vec<_>>()
            )));
        }
        Ok(ah_contracts::team_schema::storage_merged_params(
            r#type, params,
        ))
    }
}

/// team-schema 插件:注册 `team-schema`(校验/模型门面)与 `infra-registry`。
pub struct TeamSchemaPlugin;

impl Plugin for TeamSchemaPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-team-schema"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TEAM_SCHEMA, INFRA_REGISTRY]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        // 校验/模型门面:纯函数委托契约(类型门面无状态)。
        let svc: Arc<dyn TeamSchemaFacade> = Arc::new(TeamSchemaFacadeImpl);
        // infra 注册表:真实可逆注册 + 内置惰性初始化。
        let registry: Arc<dyn InfraRegistry> = Arc::new(InfraRegistryImpl::new());
        let mut effects = vec![
            ctx.register(TEAM_SCHEMA, svc),
            ctx.register(INFRA_REGISTRY, registry.clone()),
        ];
        // 挂载时即播种内置类型(对齐模块导入时注册内置 infra)。
        effects.extend(registry.ensure_builtin());
        Ok(effects)
    }
}

/// team-schema 门面 Seam(Service Definition):装配期校验 + 模型构造。
pub trait TeamSchemaFacade: Seam {
    /// 校验 ssh 认证方式(至少 key_file/password/agent 其一)。
    fn validate_ssh_auth(&self, config: &SshTransportConfig) -> Result<(), SchemaError>;

    /// 构造任务变更结果。
    fn task_op_result(&self, ok: bool, reason: &str, data: Option<Value>) -> TaskOpResult;

    /// 构造任务创建结果。
    fn task_create_result(&self, task: Option<Value>, reason: &str) -> TaskCreateResult;

    /// 构造任务图批量结果。
    fn task_graph_result(&self, ok: bool, reason: &str, tasks: Vec<Value>) -> TaskGraphResult;
}

/// 纯函数门面实现:委托契约层。
pub struct TeamSchemaFacadeImpl;

impl Seam for TeamSchemaFacadeImpl {}

impl TeamSchemaFacade for TeamSchemaFacadeImpl {
    fn validate_ssh_auth(&self, config: &SshTransportConfig) -> Result<(), SchemaError> {
        validate_ssh_auth(config)
    }

    fn task_op_result(&self, ok: bool, reason: &str, data: Option<Value>) -> TaskOpResult {
        if ok {
            TaskOpResult::success(data)
        } else {
            TaskOpResult::fail(reason.to_string())
        }
    }

    fn task_create_result(&self, task: Option<Value>, reason: &str) -> TaskCreateResult {
        match task {
            Some(t) => TaskCreateResult::success(t),
            None => TaskCreateResult::fail(reason.to_string()),
        }
    }

    fn task_graph_result(&self, ok: bool, reason: &str, tasks: Vec<Value>) -> TaskGraphResult {
        if ok {
            TaskGraphResult::success(tasks)
        } else {
            TaskGraphResult::fail(reason.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{INFRA_REGISTRY, TEAM_SCHEMA};
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(TeamSchemaPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seams_registered_and_builtin_seeded() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn TeamSchemaFacade>(&TEAM_SCHEMA).is_some());
        let registry = ctx
            .service::<dyn InfraRegistry>(&INFRA_REGISTRY)
            .expect("infra-registry");
        // 内置类型在挂载时已播种。
        assert!(
            registry
                .resolve_transport("inprocess", &BTreeMap::new())
                .is_ok()
        );
        assert!(
            registry
                .resolve_transport("pyzmq", &BTreeMap::new())
                .is_ok()
        );
        assert!(registry.resolve_storage("sqlite", &BTreeMap::new()).is_ok());
        assert!(registry.resolve_storage("memory", &BTreeMap::new()).is_ok());
        drop(effects);
    }

    #[test]
    fn resolve_unknown_type_errs_and_merged_params() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn InfraRegistry>(&INFRA_REGISTRY)
            .expect("infra-registry");
        let err = registry
            .resolve_transport("nope", &BTreeMap::new())
            .unwrap_err();
        assert!(err.0.contains("Unknown transport type 'nope'"), "{err}");
        let err2 = registry
            .resolve_storage("nope", &BTreeMap::new())
            .unwrap_err();
        assert!(err2.0.contains("Unknown storage type 'nope'"), "{err2}");
        // backend 键注入。
        let v = registry
            .resolve_transport("hybrid", &BTreeMap::new())
            .expect("hybrid");
        assert_eq!(v["backend"], "hybrid");
        let sv = registry
            .resolve_storage("mysql", &BTreeMap::new())
            .expect("mysql");
        assert_eq!(sv["db_type"], "mysql");
        drop(effects);
    }

    #[test]
    fn register_is_reversible() {
        let (ctx, effects) = build_ctx();
        let registry = ctx
            .service::<dyn InfraRegistry>(&INFRA_REGISTRY)
            .expect("infra-registry");
        // 注册自定义 transport → 可解析。
        let reg = registry.register_transport("custom-t");
        assert!(
            registry
                .resolve_transport("custom-t", &BTreeMap::new())
                .is_ok()
        );
        drop(reg);
        // drop 后回滚 → 显式 Err。
        let err = registry
            .resolve_transport("custom-t", &BTreeMap::new())
            .unwrap_err();
        assert!(err.0.contains("Unknown transport type 'custom-t'"), "{err}");
        drop(effects);
    }

    #[test]
    fn facade_validates_and_constructs() {
        let (ctx, effects) = build_ctx();
        let facade = ctx
            .service::<dyn TeamSchemaFacade>(&TEAM_SCHEMA)
            .expect("team-schema");
        // ssh 认证校验。
        let bad = SshTransportConfig {
            host: "h".to_string(),
            port: 22,
            username: None,
            key_file: None,
            password: None,
            agent: false,
            known_hosts: None,
            disable_host_key_check: false,
            connect_timeout_s: 15.0,
        };
        assert!(facade.validate_ssh_auth(&bad).is_err());
        let mut ok = bad.clone();
        ok.agent = true;
        assert!(facade.validate_ssh_auth(&ok).is_ok());
        // 模型构造。
        let op = facade.task_op_result(true, "", Some(json!({"x": 1})));
        assert!(op.ok);
        let create = facade.task_create_result(None, "bad title");
        assert!(!create.ok());
        assert_eq!(create.reason, "bad title");
        let graph = facade.task_graph_result(false, "cycle", vec![]);
        assert!(!graph.ok);
        assert_eq!(graph.reason, "cycle");
        drop(effects);
    }
}
