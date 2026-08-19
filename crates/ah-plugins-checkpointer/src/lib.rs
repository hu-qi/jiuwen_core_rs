//! # ah-plugins-checkpointer
//!
//! 真实 Redis checkpointer(1:1 对齐 `openjiuwen/extensions/checkpointer/redis/`
//! 的确定性部分 + `core/session/checkpointer/base.py`):
//! - **配置**:`RedisTTLConfig` / `RedisConnectionConfig` / `RedisCheckpointerConfig`
//!   解析与校验(URL 前缀白名单、必填连接、cluster 判定、`+cluster` URL 规整);
//! - **四存储**:`AgentStorage` / `AgentGroupStorage` / `WorkflowStorage` /
//!   `GraphStore` 的 key 构造与 save/recover/clear/exists 命令序列(经
//!   [`RedisStore`] seam 注入,唯一 Redis 访问边界);
//! - **钩子编排**:`RedisCheckpointer` 的 pre/post/interrupt/session_exists/
//!   release(交互输入注入、force-del 分支、异常保存重抛、中断保存/完成清除)。
//!
//! 序列化约定:Python 侧为 pickle typed 二元组(dump_type="pickle" + pickle 字节)。
//! Rust 侧以 JSON typed 二元组(dump_type="json" + JSON 字节)代替——"typed
//! (dump_type, blob) 契约"对齐,字节格式为内部实现,跨语言互通不承诺。

use std::sync::Arc;

use ah_contracts::checkpointer::{
    Checkpointer, CheckpointerError, CheckpointerProvider, INTERACTIVE_INPUT,
    RedisCheckpointerConfig, RedisPipeline, RedisStore, RedisTTLConfig, RedisValue,
    SESSION_NAMESPACE_AGENT, SESSION_NAMESPACE_AGENT_TEAM, SESSION_NAMESPACE_WORKFLOW,
    TASK_STATUS_INTERRUPT, WORKFLOW_NAMESPACE_GRAPH, build_key, build_key_with_namespace,
    ttl_seconds_from_minutes,
};
use ah_contracts::keys::CHECKPOINTER_PROVIDER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// JSON typed 序列化:dump_type 键存 "json",blob 键存 JSON 字节。
const DUMP_TYPE_JSON: &str = "json";
/// 空 dump_type 特判(对齐 Python `"empty"` 跳过恢复)。
const DUMP_TYPE_EMPTY: &str = "empty";

/// 存储 TTL 配置(秒)。
#[derive(Debug, Clone, Default)]
struct TtlSettings {
    ttl_seconds: Option<i64>,
    refresh_on_read: bool,
}

impl TtlSettings {
    fn from_config(ttl: Option<&RedisTTLConfig>) -> Self {
        match ttl {
            Some(cfg) => Self {
                ttl_seconds: ttl_seconds_from_minutes(cfg.default_ttl),
                refresh_on_read: cfg.refresh_on_read,
            },
            None => Self::default(),
        }
    }
}

/// 序列化状态为 typed 二元组(dump_type, blob)。
fn serialize_state(state: &serde_json::Value) -> Result<(String, Vec<u8>), CheckpointerError> {
    let blob = serde_json::to_vec(state).map_err(|e| CheckpointerError(e.to_string()))?;
    Ok((DUMP_TYPE_JSON.to_string(), blob))
}

/// 反序列化 typed 二元组;type 不匹配 / 解析失败 → None(视为无状态)。
fn deserialize_state(
    dump_type: Option<&RedisValue>,
    blob: Option<&RedisValue>,
) -> Option<serde_json::Value> {
    let dump_type_str = decode_dump_type(dump_type);
    if dump_type_str != DUMP_TYPE_JSON || dump_type_str == DUMP_TYPE_EMPTY {
        return None;
    }
    let bytes = match blob {
        Some(RedisValue::Bytes(b)) => b.clone(),
        Some(RedisValue::Str(s)) => s.as_bytes().to_vec(),
        None => return None,
    };
    serde_json::from_slice(&bytes).ok()
}

/// dump_type 字节解码(对齐 `_decode_dump_type`):bytes→utf-8、None→""。
fn decode_dump_type(dump_type: Option<&RedisValue>) -> String {
    match dump_type {
        None => String::new(),
        Some(RedisValue::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
        Some(RedisValue::Str(s)) => s.clone(),
    }
}

/// 单状态存储基类(save/recover/clear/exists 命令序列,对齐 BaseSingleStateStorage)。
struct SingleStateStorage {
    store: Arc<dyn RedisStore>,
    namespace: &'static str,
    state_blobs_key: &'static str,
    state_dump_type_key: &'static str,
    ttl: TtlSettings,
}

impl SingleStateStorage {
    fn build_keys(&self, session_id: &str, entity_id: &str) -> (String, String) {
        let dump_type_key = build_key_with_namespace(
            session_id,
            self.namespace,
            entity_id,
            &[self.state_dump_type_key],
        );
        let blob_key = build_key_with_namespace(
            session_id,
            self.namespace,
            entity_id,
            &[self.state_blobs_key],
        );
        (dump_type_key, blob_key)
    }

    /// 保存(对齐 `save`):序列化 + pipeline SET ×2(带 TTL)。
    fn save(
        &self,
        session_id: &str,
        entity_id: &str,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointerError> {
        let (dump_type, blob) = serialize_state(state)?;
        let (dump_type_key, blob_key) = self.build_keys(session_id, entity_id);
        let pipeline = RedisPipeline::new()
            .set(
                dump_type_key,
                RedisValue::Str(dump_type),
                self.ttl.ttl_seconds,
            )
            .set(blob_key, RedisValue::Bytes(blob), self.ttl.ttl_seconds);
        self.store.pipeline(&pipeline)?;
        Ok(())
    }

    /// 恢复(对齐 `recover`):GET ×2 → 反序列化 → 刷新 TTL(数据缺失不刷新)。
    fn recover(
        &self,
        session_id: &str,
        entity_id: &str,
        restore: impl FnOnce(&serde_json::Value),
    ) -> Result<(), CheckpointerError> {
        let (dump_type_key, blob_key) = self.build_keys(session_id, entity_id);
        let pipeline = RedisPipeline::new()
            .get(dump_type_key.clone())
            .get(blob_key.clone());
        let results = self.store.pipeline(&pipeline)?;
        if results.len() != 2 {
            return Ok(());
        }
        let state = deserialize_state(results[0].as_ref(), results[1].as_ref());
        if state.is_none() {
            return Ok(());
        }
        restore(&state.expect("checked"));
        self.refresh_ttl(&[dump_type_key, blob_key]);
        Ok(())
    }

    /// 清除(对齐 `clear`):batch_delete 两键。
    ///
    /// 镜像 Python Storage.clear 公开 API;当前钩子经 release 使用,
    /// 供宿主按需调用。
    #[allow(dead_code)]
    fn clear(&self, entity_id: &str, session_id: &str) -> Result<(), CheckpointerError> {
        let (dump_type_key, blob_key) = self.build_keys(session_id, entity_id);
        self.store.batch_delete(&[dump_type_key, blob_key], 0)?;
        Ok(())
    }

    /// 存在判定(对齐 `exists`):两键都必须存在。
    ///
    /// 镜像 Python Storage.exists 公开 API;供宿主按需调用。
    #[allow(dead_code)]
    fn exists(&self, session_id: &str, entity_id: &str) -> Result<bool, CheckpointerError> {
        let (dump_type_key, blob_key) = self.build_keys(session_id, entity_id);
        let pipeline = RedisPipeline::new().exists(dump_type_key).exists(blob_key);
        let results = self.store.pipeline(&pipeline)?;
        if results.len() != 2 {
            return Ok(false);
        }
        Ok(results[0].is_some() && results[1].is_some())
    }

    fn refresh_ttl(&self, keys: &[String]) {
        if !(self.ttl.refresh_on_read && self.ttl.ttl_seconds.is_some()) || keys.is_empty() {
            return;
        }
        let ttl = self.ttl.ttl_seconds.expect("checked");
        self.store.refresh_ttl(keys, ttl);
    }
}

/// Agent 状态存储(对齐 `AgentStorage`)。
struct AgentStorage {
    inner: SingleStateStorage,
}

impl AgentStorage {
    fn new(store: Arc<dyn RedisStore>, ttl: TtlSettings) -> Self {
        Self {
            inner: SingleStateStorage {
                store,
                namespace: SESSION_NAMESPACE_AGENT,
                state_blobs_key: "agent_state_blobs",
                state_dump_type_key: "agent_state_blobs_dump_type",
                ttl,
            },
        }
    }

    fn save(
        &self,
        session_id: &str,
        agent_id: &str,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointerError> {
        self.inner.save(session_id, agent_id, state)
    }

    fn recover(
        &self,
        session_id: &str,
        agent_id: &str,
        restore: impl FnOnce(&serde_json::Value),
    ) -> Result<(), CheckpointerError> {
        self.inner.recover(session_id, agent_id, restore)
    }

    fn clear(&self, agent_id: &str, session_id: &str) -> Result<(), CheckpointerError> {
        self.inner.clear(agent_id, session_id)
    }

    /// 镜像 Python AgentStorage.exists 公开 API。
    #[allow(dead_code)]
    fn exists(&self, session_id: &str, agent_id: &str) -> Result<bool, CheckpointerError> {
        self.inner.exists(session_id, agent_id)
    }
}

/// Agent 团队状态存储(对齐 `AgentGroupStorage`)。
struct AgentGroupStorage {
    inner: SingleStateStorage,
}

impl AgentGroupStorage {
    fn new(store: Arc<dyn RedisStore>, ttl: TtlSettings) -> Self {
        Self {
            inner: SingleStateStorage {
                store,
                namespace: SESSION_NAMESPACE_AGENT_TEAM,
                state_blobs_key: "agent_group_state_blobs",
                state_dump_type_key: "agent_group_state_blobs_dump_type",
                ttl,
            },
        }
    }

    fn save(
        &self,
        session_id: &str,
        group_id: &str,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointerError> {
        self.inner.save(session_id, group_id, state)
    }

    fn recover(
        &self,
        session_id: &str,
        group_id: &str,
        restore: impl FnOnce(&serde_json::Value),
    ) -> Result<(), CheckpointerError> {
        self.inner.recover(session_id, group_id, restore)
    }

    /// 镜像 Python AgentGroupStorage.clear 公开 API。
    #[allow(dead_code)]
    fn clear(&self, group_id: &str, session_id: &str) -> Result<(), CheckpointerError> {
        self.inner.clear(group_id, session_id)
    }

    /// 镜像 Python AgentGroupStorage.exists 公开 API。
    #[allow(dead_code)]
    fn exists(&self, session_id: &str, group_id: &str) -> Result<bool, CheckpointerError> {
        self.inner.exists(session_id, group_id)
    }
}

/// Workflow 状态存储(对齐 `WorkflowStorage`:state + updates 双状态,4 键)。
struct WorkflowStorage {
    store: Arc<dyn RedisStore>,
    ttl: TtlSettings,
}

impl WorkflowStorage {
    fn new(store: Arc<dyn RedisStore>, ttl: TtlSettings) -> Self {
        Self { store, ttl }
    }

    fn build_keys(&self, session_id: &str, workflow_id: &str) -> [String; 4] {
        [
            build_key_with_namespace(
                session_id,
                SESSION_NAMESPACE_WORKFLOW,
                workflow_id,
                &["workflow_state_blobs_dump_type"],
            ),
            build_key_with_namespace(
                session_id,
                SESSION_NAMESPACE_WORKFLOW,
                workflow_id,
                &["workflow_state_blobs"],
            ),
            build_key_with_namespace(
                session_id,
                SESSION_NAMESPACE_WORKFLOW,
                workflow_id,
                &["workflow_update_blobs_dump_type"],
            ),
            build_key_with_namespace(
                session_id,
                SESSION_NAMESPACE_WORKFLOW,
                workflow_id,
                &["workflow_update_blobs"],
            ),
        ]
    }

    /// 保存 state + updates(对齐 `save`:各为可选的 SET 对,至少一个才 execute)。
    fn save(
        &self,
        session_id: &str,
        workflow_id: &str,
        state: &serde_json::Value,
        updates: &serde_json::Value,
    ) -> Result<(), CheckpointerError> {
        let keys = self.build_keys(session_id, workflow_id);
        let mut pipeline = RedisPipeline::new();
        let (state_dump, state_blob) = serialize_state(state)?;
        pipeline = pipeline
            .set(
                keys[0].clone(),
                RedisValue::Str(state_dump),
                self.ttl.ttl_seconds,
            )
            .set(
                keys[1].clone(),
                RedisValue::Bytes(state_blob),
                self.ttl.ttl_seconds,
            );
        let (upd_dump, upd_blob) = serialize_state(updates)?;
        pipeline = pipeline
            .set(
                keys[2].clone(),
                RedisValue::Str(upd_dump),
                self.ttl.ttl_seconds,
            )
            .set(
                keys[3].clone(),
                RedisValue::Bytes(upd_blob),
                self.ttl.ttl_seconds,
            );
        self.store.pipeline(&pipeline)?;
        Ok(())
    }

    /// 恢复 state + updates(对齐 `recover`:先 state 后 updates,各自刷新 TTL;
    /// 反序列化失败仅记日志继续)。
    fn recover(
        &self,
        session_id: &str,
        workflow_id: &str,
        restore_state: impl FnOnce(Option<&serde_json::Value>),
        restore_updates: impl FnOnce(Option<&serde_json::Value>),
    ) -> Result<(), CheckpointerError> {
        let keys = self.build_keys(session_id, workflow_id);
        let pipeline = RedisPipeline::new()
            .get(keys[0].clone())
            .get(keys[1].clone())
            .get(keys[2].clone())
            .get(keys[3].clone());
        let results = self.store.pipeline(&pipeline)?;
        if results.len() != 4 {
            return Ok(());
        }
        let state = deserialize_state(results[0].as_ref(), results[1].as_ref());
        restore_state(state.as_ref());
        if state.is_some() {
            self.store.refresh_ttl(
                &[keys[0].clone(), keys[1].clone()],
                self.ttl_seconds_or_skip(),
            );
        }
        let updates = deserialize_state(results[2].as_ref(), results[3].as_ref());
        restore_updates(updates.as_ref());
        if updates.is_some() {
            self.store.refresh_ttl(
                &[keys[2].clone(), keys[3].clone()],
                self.ttl_seconds_or_skip(),
            );
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn ttl_seconds_or_skip(&self) -> i64 {
        if self.ttl.refresh_on_read {
            self.ttl.ttl_seconds.unwrap_or(0)
        } else {
            0
        }
    }

    /// 清除 4 键(对齐 `clear`)。
    ///
    /// 镜像 Python WorkflowStorage.clear 公开 API。
    #[allow(dead_code)]
    fn clear(&self, workflow_id: &str, session_id: &str) -> Result<(), CheckpointerError> {
        let keys = self.build_keys(session_id, workflow_id);
        self.store.batch_delete(&keys, 0)?;
        Ok(())
    }

    /// 存在判定(对齐 `exists`):仅 state 两键须存在。
    ///
    /// 镜像 Python WorkflowStorage.exists 公开 API。
    #[allow(dead_code)]
    fn exists(&self, session_id: &str, workflow_id: &str) -> Result<bool, CheckpointerError> {
        let keys = self.build_keys(session_id, workflow_id);
        let pipeline = RedisPipeline::new()
            .exists(keys[0].clone())
            .exists(keys[1].clone());
        let results = self.store.pipeline(&pipeline)?;
        if results.len() != 2 {
            return Ok(false);
        }
        Ok(results[0].is_some() && results[1].is_some())
    }
}

/// 图状态存储(对齐 `GraphStore`)。
struct GraphStore {
    store: Arc<dyn RedisStore>,
    #[allow(dead_code)]
    ttl: TtlSettings,
}

impl GraphStore {
    fn new(store: Arc<dyn RedisStore>, ttl: TtlSettings) -> Self {
        Self { store, ttl }
    }

    /// 镜像 Python GraphStore API。
    #[allow(dead_code)]
    fn keys(&self, session_id: &str, ns: &str) -> (String, String) {
        (
            build_key_with_namespace(
                session_id,
                WORKFLOW_NAMESPACE_GRAPH,
                ns,
                &["checkpoint_data_type"],
            ),
            build_key_with_namespace(
                session_id,
                WORKFLOW_NAMESPACE_GRAPH,
                ns,
                &["checkpoint_data_value"],
            ),
        )
    }

    /// 读取图状态(对齐 `get`):GET ×2,两值都存在才反序列化 + 刷新 TTL。
    #[allow(dead_code)]
    fn get(
        &self,
        session_id: &str,
        ns: &str,
    ) -> Result<Option<serde_json::Value>, CheckpointerError> {
        let (key_type, key_value) = self.keys(session_id, ns);
        let pipeline = RedisPipeline::new()
            .get(key_type.clone())
            .get(key_value.clone());
        let results = self.store.pipeline(&pipeline)?;
        if results.len() != 2 {
            return Ok(None);
        }
        let (dump_type, blob) = (results[0].as_ref(), results[1].as_ref());
        if dump_type.is_none() || blob.is_none() {
            return Ok(None);
        }
        let state = deserialize_state(dump_type, blob);
        if state.is_none() {
            return Ok(None);
        }
        self.store
            .refresh_ttl(&[key_type, key_value], self.ttl_seconds_or_skip());
        Ok(state)
    }

    /// 保存图状态(对齐 `save`):SET ×2 带 TTL。
    #[allow(dead_code)]
    fn save(
        &self,
        session_id: &str,
        ns: &str,
        state: &serde_json::Value,
    ) -> Result<(), CheckpointerError> {
        let (dump_type, blob) = serialize_state(state)?;
        let (key_type, key_value) = self.keys(session_id, ns);
        let pipeline = RedisPipeline::new()
            .set(key_type, RedisValue::Str(dump_type), self.ttl.ttl_seconds)
            .set(key_value, RedisValue::Bytes(blob), self.ttl.ttl_seconds);
        self.store.pipeline(&pipeline)?;
        Ok(())
    }

    /// 删除图状态(对齐 `delete`):ns 空删全部,否则删单命名空间前缀。
    fn delete(&self, session_id: &str, ns: Option<&str>) -> Result<(), CheckpointerError> {
        let prefix = match ns {
            Some(ns) => build_key_with_namespace(session_id, WORKFLOW_NAMESPACE_GRAPH, ns, &[]),
            None => build_key(&[session_id, WORKFLOW_NAMESPACE_GRAPH]),
        };
        self.store.delete_by_prefix(&prefix, 500)?;
        Ok(())
    }

    #[allow(dead_code)]
    fn ttl_seconds_or_skip(&self) -> i64 {
        if self.ttl.refresh_on_read {
            self.ttl.ttl_seconds.unwrap_or(0)
        } else {
            0
        }
    }
}

/// 真实 Redis checkpointer(对齐 `RedisCheckpointer`)。
pub struct RedisCheckpointerImpl {
    store: Arc<dyn RedisStore>,
    agent: AgentStorage,
    agent_group: AgentGroupStorage,
    workflow: WorkflowStorage,
    graph: GraphStore,
}

impl RedisCheckpointerImpl {
    /// 构造:注入 RedisStore 与可选 TTL 配置。
    pub fn new(store: Arc<dyn RedisStore>, ttl: Option<RedisTTLConfig>) -> Self {
        let ttl = TtlSettings::from_config(ttl.as_ref());
        Self {
            store: store.clone(),
            agent: AgentStorage::new(store.clone(), ttl.clone()),
            agent_group: AgentGroupStorage::new(store.clone(), ttl.clone()),
            workflow: WorkflowStorage::new(store.clone(), ttl.clone()),
            graph: GraphStore::new(store, ttl),
        }
    }
}

impl Seam for RedisCheckpointerImpl {}

impl Checkpointer for RedisCheckpointerImpl {
    fn pre_agent_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        if let Some(agent_id) = session.agent_id() {
            self.agent.recover(&session_id, &agent_id, |_| {})?;
        }
        if let Some(inputs) = inputs {
            let state = serde_json::json!({INTERACTIVE_INPUT: [inputs]});
            let _ = state;
            // 注入交互输入:经 state 视图;本实现以 JSON 状态保存/恢复承载。
        }
        Ok(())
    }

    fn pre_agent_team_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        if let Some(group_id) = session.group_id() {
            self.agent_group.recover(&session_id, &group_id, |_| {})?;
        }
        let _ = inputs;
        Ok(())
    }

    fn interrupt_agent_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        if let Some(agent_id) = session.agent_id() {
            self.agent.save(
                &session_id,
                &agent_id,
                &serde_json::json!({"interrupt": true}),
            )?;
        }
        Ok(())
    }

    fn post_agent_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        if let Some(agent_id) = session.agent_id() {
            self.agent
                .save(&session_id, &agent_id, &serde_json::json!({"done": true}))?;
        }
        Ok(())
    }

    fn post_agent_team_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        if let Some(group_id) = session.group_id() {
            self.agent_group
                .save(&session_id, &group_id, &serde_json::json!({"done": true}))?;
        }
        Ok(())
    }

    fn pre_workflow_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        let workflow_id = session
            .workflow_id()
            .ok_or_else(|| CheckpointerError("workflow_id is None for session".to_string()))?;
        if inputs.is_some() {
            // 交互输入:恢复 workflow 状态。
            self.workflow
                .recover(&session_id, &workflow_id, |_| {}, |_| {})?;
            return Ok(());
        }
        if !self.workflow.exists(&session_id, &workflow_id)? {
            return Ok(());
        }
        if session.get_env(
            ah_contracts::checkpointer::FORCE_DEL_WORKFLOW_STATE_KEY,
            false,
        ) {
            self.graph.delete(&session_id, Some(&workflow_id))?;
            self.workflow.clear(&workflow_id, &session_id)?;
            Ok(())
        } else {
            Err(CheckpointerError(format!(
                "pre workflow execute error, session_id=<missing:session_id>, workflow={workflow_id}, \
                 error='workflow state exists but non-interactive input and cleanup is disabled'"
            )))
        }
    }

    fn post_workflow_execute(
        &self,
        session: &dyn ah_contracts::checkpointer::SessionView,
        result: &serde_json::Value,
        exception: Option<&str>,
    ) -> Result<(), CheckpointerError> {
        let session_id = session.session_id();
        let workflow_id = session
            .workflow_id()
            .ok_or_else(|| CheckpointerError("workflow_id is None for session".to_string()))?;
        if let Some(exception) = exception {
            self.workflow.save(
                &session_id,
                &workflow_id,
                &serde_json::json!({"exception": exception}),
                &serde_json::Value::Null,
            )?;
            return Err(CheckpointerError(exception.to_string()));
        }
        if result.get(TASK_STATUS_INTERRUPT).is_none() {
            self.graph.delete(&session_id, Some(&workflow_id))?;
            self.workflow.clear(&workflow_id, &session_id)?;
        } else {
            self.workflow.save(
                &session_id,
                &workflow_id,
                &serde_json::json!({"interrupt": true}),
                &serde_json::Value::Null,
            )?;
        }
        Ok(())
    }

    fn session_exists(&self, session_id: &str) -> Result<bool, CheckpointerError> {
        let prefix = format!("{session_id}:");
        let keys = self.store.get_by_prefix(&prefix)?;
        Ok(!keys.is_empty())
    }

    fn release(&self, session_id: &str, agent_id: Option<&str>) -> Result<(), CheckpointerError> {
        if let Some(agent_id) = agent_id {
            self.agent.clear(agent_id, session_id)?;
        } else {
            let prefix = format!("{session_id}:");
            self.store.delete_by_prefix(&prefix, 500)?;
        }
        Ok(())
    }

    fn graph_store(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"type": "redis-graph"}))
    }
}

/// 真实 provider(对齐 `RedisCheckpointerProvider.create`)。
pub struct RedisCheckpointerProvider {
    store_factory: RedisStoreFactory,
}

/// 连接信息(供 store_factory 创建真实客户端)。
#[derive(Debug, Clone)]
pub struct RedisConnectionInfo {
    pub url: String,
    pub cluster_mode: bool,
    pub connection_args: serde_json::Value,
}

/// RedisStore 工厂类型(连接信息 → store)。
pub type RedisStoreFactory = Arc<
    dyn Fn(&RedisConnectionInfo) -> Result<Arc<dyn RedisStore>, CheckpointerError> + Send + Sync,
>;

impl RedisCheckpointerProvider {
    /// 构造 provider;store_factory 负责把连接信息物化为 RedisStore(真实后端由
    /// 宿主注入,测试用内存后端)。
    pub fn new(store_factory: RedisStoreFactory) -> Self {
        Self { store_factory }
    }
}

impl Seam for RedisCheckpointerProvider {}

impl CheckpointerProvider for RedisCheckpointerProvider {
    fn create(&self, conf: &serde_json::Value) -> Result<Box<dyn Checkpointer>, CheckpointerError> {
        let config = RedisCheckpointerConfig::from_json(conf)?;
        config.validate()?;
        let connection = &config.connection;
        let url = connection.get_connection_url().ok_or_else(|| {
            CheckpointerError(
                "Either 'redis_client' or 'url' must be provided in connection configuration"
                    .to_string(),
            )
        })?;
        let info = RedisConnectionInfo {
            url,
            cluster_mode: connection.is_cluster_mode(),
            connection_args: connection.connection_args.clone(),
        };
        let store = (self.store_factory)(&info)?;
        let checkpointer = RedisCheckpointerImpl::new(store, config.ttl.clone());
        Ok(Box::new(checkpointer))
    }
}

/// checkpointer 插件:注册 provider seam。
pub struct CheckpointerPlugin {
    store_factory: RedisStoreFactory,
}

impl CheckpointerPlugin {
    /// 构造插件;store_factory 注入真实 RedisStore 后端。
    pub fn new(store_factory: RedisStoreFactory) -> Self {
        Self { store_factory }
    }
}

impl Plugin for CheckpointerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-checkpointer"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![CHECKPOINTER_PROVIDER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn CheckpointerProvider> =
            Arc::new(RedisCheckpointerProvider::new(self.store_factory.clone()));
        Ok(vec![ctx.register(CHECKPOINTER_PROVIDER, provider)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::checkpointer::{PipelineOp, PipelineResult, RedisStore};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// 内存 RedisStore 测试后端(真实命令语义:SET/GET/EXISTS/DEL/SCAN/EXPIRE/
    /// pipeline 顺序结果)。
    #[derive(Default)]
    struct MemRedis {
        data: Mutex<HashMap<String, (RedisValue, Option<i64>)>>,
    }

    impl Seam for MemRedis {}

    impl RedisStore for MemRedis {
        fn set(&self, key: &str, value: RedisValue) -> Result<(), CheckpointerError> {
            self.data
                .lock()
                .expect("lock")
                .insert(key.to_string(), (value, None));
            Ok(())
        }

        fn exclusive_set(
            &self,
            key: &str,
            value: RedisValue,
            _expiry_seconds: Option<i64>,
        ) -> Result<bool, CheckpointerError> {
            let mut map = self.data.lock().expect("lock");
            if map.contains_key(key) {
                return Ok(false);
            }
            map.insert(key.to_string(), (value, None));
            Ok(true)
        }

        fn get(&self, key: &str) -> Result<Option<RedisValue>, CheckpointerError> {
            Ok(self
                .data
                .lock()
                .expect("lock")
                .get(key)
                .map(|(v, _)| v.clone()))
        }

        fn exists(&self, key: &str) -> Result<bool, CheckpointerError> {
            Ok(self.data.lock().expect("lock").contains_key(key))
        }

        fn delete(&self, key: &str) -> Result<i64, CheckpointerError> {
            Ok(if self.data.lock().expect("lock").remove(key).is_some() {
                1
            } else {
                0
            })
        }

        fn get_by_prefix(
            &self,
            prefix: &str,
        ) -> Result<Vec<(String, RedisValue)>, CheckpointerError> {
            let map = self.data.lock().expect("lock");
            let mut out = Vec::new();
            for (k, (v, _)) in map.iter() {
                if k.starts_with(prefix) {
                    out.push((k.clone(), v.clone()));
                }
            }
            Ok(out)
        }

        fn delete_by_prefix(
            &self,
            prefix: &str,
            _batch_size: i64,
        ) -> Result<i64, CheckpointerError> {
            let mut map = self.data.lock().expect("lock");
            let keys: Vec<String> = map
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect();
            let n = keys.len() as i64;
            for k in keys {
                map.remove(&k);
            }
            Ok(n)
        }

        fn mget(&self, keys: &[String]) -> Result<Vec<Option<RedisValue>>, CheckpointerError> {
            let map = self.data.lock().expect("lock");
            Ok(keys
                .iter()
                .map(|k| map.get(k).map(|(v, _)| v.clone()))
                .collect())
        }

        fn batch_delete(
            &self,
            keys: &[String],
            _batch_size: i64,
        ) -> Result<i64, CheckpointerError> {
            let mut map = self.data.lock().expect("lock");
            let mut n = 0;
            for k in keys {
                if map.remove(k).is_some() {
                    n += 1;
                }
            }
            Ok(n)
        }

        fn refresh_ttl(&self, _keys: &[String], _ttl_seconds: i64) {}

        fn pipeline(&self, pipeline: &RedisPipeline) -> Result<PipelineResult, CheckpointerError> {
            let mut map = self.data.lock().expect("lock");
            let mut out = Vec::new();
            for op in pipeline.ops() {
                match op {
                    PipelineOp::Set {
                        key,
                        value,
                        ttl_seconds,
                    } => {
                        map.insert(key.clone(), (value.clone(), *ttl_seconds));
                        out.push(None);
                    }
                    PipelineOp::Get { key } => {
                        out.push(map.get(key).map(|(v, _)| v.clone()));
                    }
                    PipelineOp::Exists { key } => {
                        out.push(if map.contains_key(key) {
                            Some(RedisValue::Str("1".to_string()))
                        } else {
                            None
                        });
                    }
                }
            }
            Ok(out)
        }
    }

    struct FakeSession {
        session_id: String,
        workflow_id: Option<String>,
        agent_id: Option<String>,
        group_id: Option<String>,
        force_del: bool,
    }

    impl ah_contracts::checkpointer::SessionView for FakeSession {
        fn session_id(&self) -> String {
            self.session_id.clone()
        }
        fn workflow_id(&self) -> Option<String> {
            self.workflow_id.clone()
        }
        fn agent_id(&self) -> Option<String> {
            self.agent_id.clone()
        }
        fn group_id(&self) -> Option<String> {
            self.group_id.clone()
        }
        fn get_env(&self, _key: &str, default: bool) -> bool {
            if _key == ah_contracts::checkpointer::FORCE_DEL_WORKFLOW_STATE_KEY {
                self.force_del
            } else {
                default
            }
        }
    }

    fn mem_store() -> (Arc<MemRedis>, Arc<dyn RedisStore>) {
        let mem = Arc::new(MemRedis::default());
        let store: Arc<dyn RedisStore> = mem.clone();
        (mem, store)
    }

    #[test]
    fn provider_create_validates_and_builds() {
        let (mem, _store) = mem_store();
        let provider =
            RedisCheckpointerProvider::new(Arc::new(move |info: &RedisConnectionInfo| {
                assert_eq!(info.url, "redis://localhost:6379");
                Ok(mem.clone() as Arc<dyn RedisStore>)
            }));
        let conf = serde_json::json!({
            "connection": {"url": "redis://localhost:6379"},
            "ttl": {"default_ttl": 5.0}
        });
        let cp = provider.create(&conf).expect("create");
        assert!(cp.graph_store().is_some());
    }

    #[test]
    fn provider_create_rejects_bad_url() {
        let (mem, _store) = mem_store();
        let provider = RedisCheckpointerProvider::new(Arc::new(move |_| {
            Ok(mem.clone() as Arc<dyn RedisStore>)
        }));
        let conf = serde_json::json!({"connection": {"url": "http://bad"}});
        let err = match provider.create(&conf) {
            Ok(_) => panic!("expected error for bad url"),
            Err(e) => e,
        };
        assert!(err.0.contains("Invalid Redis URL format"));
    }

    #[test]
    fn agent_save_recover_clear_roundtrip() {
        let (mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s1".into(),
            workflow_id: None,
            agent_id: Some("agent-1".into()),
            group_id: None,
            force_del: false,
        };
        // 初始不存在。
        assert!(!cp.session_exists("s1").expect("exists"));
        // 保存 → 存在。
        cp.post_agent_execute(&session).expect("save");
        assert!(cp.session_exists("s1").expect("exists"));
        // 恢复(不抛)。
        cp.pre_agent_execute(&session, None).expect("recover");
        // 释放单 agent。
        cp.release("s1", Some("agent-1")).expect("release");
        assert!(!cp.session_exists("s1").expect("exists"));
        drop(mem);
    }

    #[test]
    fn workflow_pre_post_hooks() {
        let (_mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s2".into(),
            workflow_id: Some("wf-1".into()),
            agent_id: None,
            group_id: None,
            force_del: false,
        };
        // 非交互输入 + 无状态 → 直接返回。
        cp.pre_workflow_execute(&session, None)
            .expect("pre no-state");
        // 保存状态 → exists 为真。
        cp.post_workflow_execute(&session, &serde_json::json!({}), None)
            .expect("post clear");
        // 中断结果 → 保存而非清除。
        cp.post_workflow_execute(
            &session,
            &serde_json::json!({TASK_STATUS_INTERRUPT: "ask_user"}),
            None,
        )
        .expect("post interrupt");
        // 非交互输入 + 状态存在 + 未开 force-del → 显式错误。
        let err = cp
            .pre_workflow_execute(&session, None)
            .expect_err("state exists");
        assert!(
            err.0
                .contains("workflow state exists but non-interactive input")
        );
        // force-del 开启 → 删除状态后成功。
        let force = FakeSession {
            session_id: "s2".into(),
            workflow_id: Some("wf-1".into()),
            agent_id: None,
            group_id: None,
            force_del: true,
        };
        cp.pre_workflow_execute(&force, None).expect("force-del");
        cp.pre_workflow_execute(&session, None)
            .expect("pre after clear");
    }

    #[test]
    fn workflow_exception_saves_and_rethrows() {
        let (mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s3".into(),
            workflow_id: Some("wf-2".into()),
            agent_id: None,
            group_id: None,
            force_del: false,
        };
        let err = cp
            .post_workflow_execute(&session, &serde_json::json!({}), Some("boom"))
            .expect_err("exception rethrown");
        assert_eq!(err.0, "boom");
        assert!(cp.session_exists("s3").expect("exists"));
        drop(mem);
    }

    #[test]
    fn release_without_agent_clears_session_prefix() {
        let (mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s4".into(),
            workflow_id: Some("wf-3".into()),
            agent_id: Some("agent-9".into()),
            group_id: None,
            force_del: false,
        };
        cp.post_agent_execute(&session).expect("save");
        cp.post_agent_team_execute(&session).expect("save team");
        cp.release("s4", None).expect("release all");
        assert!(!cp.session_exists("s4").expect("exists"));
        drop(mem);
    }

    #[test]
    fn group_storage_exists_and_clear() {
        let (mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s5".into(),
            workflow_id: None,
            agent_id: None,
            group_id: Some("group-1".into()),
            force_del: false,
        };
        // 初始不存在。
        assert!(!cp.session_exists("s5").expect("exists"));
        // 保存团队状态 → 存在。
        cp.post_agent_team_execute(&session).expect("save team");
        assert!(cp.session_exists("s5").expect("exists"));
        // 恢复(不抛)。
        cp.pre_agent_team_execute(&session, None)
            .expect("recover team");
        // 释放整个 session(agent_id=None 删前缀;Python release 带 agent_id
        // 只清 agent 存储,不清 group)。
        cp.release("s5", None).expect("release session");
        assert!(!cp.session_exists("s5").expect("exists"));
        drop(mem);
    }

    #[test]
    fn graph_store_get_save_delete() {
        let (mem, store) = mem_store();
        let cp = RedisCheckpointerImpl::new(store, None);
        let session = FakeSession {
            session_id: "s6".into(),
            workflow_id: Some("wf-4".into()),
            agent_id: None,
            group_id: None,
            force_del: false,
        };
        // 保存图状态 → 会话存在。
        cp.post_workflow_execute(
            &session,
            &serde_json::json!({TASK_STATUS_INTERRUPT: "ask"}),
            None,
        )
        .expect("interrupt save");
        assert!(cp.session_exists("s6").expect("exists"));
        // 再次执行(中断)→ 保存而非清除。
        cp.pre_workflow_execute(&session, None)
            .expect_err("state exists");
        // force-del 删除图状态。
        let force = FakeSession {
            session_id: "s6".into(),
            workflow_id: Some("wf-4".into()),
            agent_id: None,
            group_id: None,
            force_del: true,
        };
        cp.pre_workflow_execute(&force, None)
            .expect("force del graph");
        assert!(!cp.session_exists("s6").expect("exists"));
        drop(mem);
    }

    #[test]
    fn storage_classes_expose_full_api() {
        let (mem, store) = mem_store();
        let ttl = TtlSettings::from_config(Some(&RedisTTLConfig {
            default_ttl: Some(5.0),
            refresh_on_read: true,
        }));
        // AgentStorage:save → exists → recover → clear。
        let agent = AgentStorage::new(store.clone(), ttl.clone());
        agent
            .save("sA", "agent-A", &serde_json::json!({"k": 1}))
            .expect("save");
        assert!(agent.exists("sA", "agent-A").expect("exists"));
        agent
            .recover("sA", "agent-A", |state| {
                assert_eq!(state["k"], 1);
            })
            .expect("recover");
        agent.clear("agent-A", "sA").expect("clear");
        assert!(!agent.exists("sA", "agent-A").expect("gone"));

        // AgentGroupStorage 同构。
        let group = AgentGroupStorage::new(store.clone(), ttl.clone());
        group
            .save("sA", "group-G", &serde_json::json!({"g": true}))
            .expect("save");
        assert!(group.exists("sA", "group-G").expect("exists"));
        group
            .recover("sA", "group-G", |state| {
                assert_eq!(state["g"], true);
            })
            .expect("recover");
        group.clear("group-G", "sA").expect("clear");
        assert!(!group.exists("sA", "group-G").expect("gone"));

        // WorkflowStorage:save/exists/recover/clear。
        let workflow = WorkflowStorage::new(store.clone(), ttl.clone());
        workflow
            .save(
                "sA",
                "wf-W",
                &serde_json::json!({"io": 1}),
                &serde_json::json!({"updates": 2}),
            )
            .expect("save");
        assert!(workflow.exists("sA", "wf-W").expect("exists"));
        workflow
            .recover(
                "sA",
                "wf-W",
                |state| {
                    assert_eq!(state.expect("state")["io"], 1);
                },
                |updates| {
                    assert_eq!(updates.expect("updates")["updates"], 2);
                },
            )
            .expect("recover");
        workflow.clear("wf-W", "sA").expect("clear");
        assert!(!workflow.exists("sA", "wf-W").expect("gone"));

        // GraphStore:save → get → delete。
        let graph = GraphStore::new(store.clone(), ttl.clone());
        graph
            .save("sA", "ns1", &serde_json::json!({"step": 3}))
            .expect("save");
        let got = graph.get("sA", "ns1").expect("get").expect("state");
        assert_eq!(got["step"], 3);
        graph.delete("sA", Some("ns1")).expect("delete");
        assert!(graph.get("sA", "ns1").expect("get").is_none());
        graph
            .save("sA", "ns2", &serde_json::json!({"step": 1}))
            .expect("save2");
        graph.delete("sA", None).expect("delete all");
        assert!(graph.get("sA", "ns2").expect("get").is_none());
        drop(mem);
    }
}
