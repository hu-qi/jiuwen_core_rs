//! checkpointer seam:Redis checkpointer 的确定性契约。
//!
//! 对齐 `openjiuwen/extensions/checkpointer/redis/{checkpointer,storage}.py`
//! 的确定性部分 + `core/session/checkpointer/base.py`:
//! - 命名空间常量 / `build_key` / `build_key_with_namespace`(key 拼接纯函数);
//! - `RedisTTLConfig` / `RedisConnectionConfig` / `RedisCheckpointerConfig`
//!   (字段、URL 前缀校验、必填校验、`is_cluster_mode` / `get_connection_url`);
//! - `Checkpointer` / `Storage` 抽象接口 + `CheckpointerProvider` 工厂接口;
//! - `RedisStore` 抽象(唯一 Redis 访问边界):SET/GET/EXISTS/DEL/MGET/SCAN/
//!   EXPIRE/pipeline/batch_delete/delete_by_prefix/refresh_ttl。
//!
//! 契约零实现:存储编排(Agent/AgentGroup/Workflow/Graph)与钩子流程由插件提供。

use crate::seam::Seam;

// ---------------------------------------------------------------------------
// 命名空间常量(对齐 core/session/checkpointer/base.py:78-86)
// ---------------------------------------------------------------------------

/// 会话下 agent 状态命名空间。
pub const SESSION_NAMESPACE_AGENT: &str = "agent";
/// 会话下 agent 团队状态命名空间(注意是连字符)。
pub const SESSION_NAMESPACE_AGENT_TEAM: &str = "agent-team";
/// 会话下 workflow 自身状态命名空间。
pub const SESSION_NAMESPACE_WORKFLOW: &str = "workflow";
/// workflow 下图状态命名空间(与 workflow 自身状态分离)。
pub const WORKFLOW_NAMESPACE_GRAPH: &str = "workflow-graph";

/// 交互输入状态键(对齐 constant.py:15 `__interactive_input__`)。
pub const INTERACTIVE_INPUT: &str = "__interactive_input__";
/// workflow 中断状态键(对齐 pregel/constants.py:10 `__interrupt__`)。
pub const TASK_STATUS_INTERRUPT: &str = "__interrupt__";
/// 强制删除 workflow 状态的环境键(对齐 session/constants.py:22)。
pub const FORCE_DEL_WORKFLOW_STATE_KEY: &str = "_force_del_workflow_state";

/// TTL 配置键(对齐 storage.py:36-38)。
pub const TTL_KEY_DEFAULT_TTL: &str = "default_ttl";
pub const TTL_KEY_REFRESH_ON_READ: &str = "refresh_on_read";
pub const SECONDS_PER_MINUTE: u64 = 60;

/// checkpointer 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointerError(pub String);

impl core::fmt::Display for CheckpointerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CheckpointerError {}

/// 拼接 Redis key(对齐 `build_key`:`":".join(parts)`)。
pub fn build_key(parts: &[&str]) -> String {
    parts.join(":")
}

/// 命名空间 key 构造(对齐 `build_key_with_namespace`)。
///
/// 参数顺序固定:`session_id → namespace → entity_id → suffixes`。
pub fn build_key_with_namespace(
    session_id: &str,
    namespace: &str,
    entity_id: &str,
    suffixes: &[&str],
) -> String {
    let mut parts = vec![session_id, namespace, entity_id];
    parts.extend_from_slice(suffixes);
    build_key(&parts)
}

/// TTL 秒换算(对齐 storage.py:60-61 `int(default_ttl * 60)` 截断)。
///
/// 传入 default_ttl(分钟,float);None 表示未配置(返回 None)。
pub fn ttl_seconds_from_minutes(default_ttl: Option<f64>) -> Option<i64> {
    default_ttl.map(|minutes| (minutes * SECONDS_PER_MINUTE as f64) as i64)
}

/// 解析 refresh_on_read 标志(对齐 storage.py:62-63 键存在即真)。
pub fn refresh_on_read_from_ttl(ttl: Option<&serde_json::Map<String, serde_json::Value>>) -> bool {
    ttl.map(|m| m.contains_key(TTL_KEY_REFRESH_ON_READ))
        .unwrap_or(false)
}

/// Redis TTL 配置(对齐 `RedisTTLConfig`)。`default_ttl` 单位:分钟。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RedisTTLConfig {
    #[serde(default)]
    pub default_ttl: Option<f64>,
    #[serde(default)]
    pub refresh_on_read: bool,
}

impl RedisTTLConfig {
    /// 从 JSON 映射解析(对齐 pydantic `model_validate`;`default_ttl` 缺省 None)。
    pub fn from_json(value: &serde_json::Value) -> Result<Self, CheckpointerError> {
        let obj = value
            .as_object()
            .ok_or_else(|| CheckpointerError("ttl must be an object".to_string()))?;
        let default_ttl = match obj.get(TTL_KEY_DEFAULT_TTL) {
            Some(v) if v.is_number() => v.as_f64(),
            Some(v) if v.is_null() => None,
            Some(_) => {
                return Err(CheckpointerError(
                    "default_ttl must be a number or null".to_string(),
                ));
            }
            None => None,
        };
        let refresh_on_read = obj
            .get(TTL_KEY_REFRESH_ON_READ)
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);
        Ok(Self {
            default_ttl,
            refresh_on_read,
        })
    }
}

/// Redis 连接配置(对齐 `RedisConnectionConfig`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RedisConnectionConfig {
    #[serde(default)]
    pub url: Option<String>,
    /// 显式集群开关(None = 从 URL scheme 自动检测)。
    #[serde(default)]
    pub cluster_mode: Option<bool>,
    /// 额外连接参数(透传给客户端创建;Rust 侧为 key→value JSON)。
    #[serde(default)]
    pub connection_args: serde_json::Value,
}

impl Default for RedisConnectionConfig {
    fn default() -> Self {
        Self {
            url: None,
            cluster_mode: None,
            connection_args: serde_json::Value::Object(Default::default()),
        }
    }
}

impl RedisConnectionConfig {
    /// 校验 URL 前缀(对齐 field_validator:redis:// / rediss:// / redis+cluster:// /
    /// rediss+cluster://)。
    pub fn validate_url(&self) -> Result<(), CheckpointerError> {
        if let Some(url) = &self.url {
            let ok = url.starts_with("redis://")
                || url.starts_with("rediss://")
                || url.starts_with("redis+cluster://")
                || url.starts_with("rediss+cluster://");
            if !ok {
                return Err(CheckpointerError(format!(
                    "Invalid Redis URL format: {url}. URL must start with redis://, rediss://, \
                     redis+cluster://, or rediss+cluster://"
                )));
            }
        }
        Ok(())
    }

    /// 至少一种连接方式(对齐 model_validator:url 必填)。
    pub fn validate_present(&self) -> Result<(), CheckpointerError> {
        if self.url.is_none() {
            return Err(CheckpointerError(
                "Either 'redis_client' or 'url' must be provided in RedisConnectionConfig"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// 是否集群模式(对齐 `is_cluster_mode` 三分支,无 redis_client 分支)。
    pub fn is_cluster_mode(&self) -> bool {
        if let Some(flag) = self.cluster_mode {
            return flag;
        }
        if let Some(url) = &self.url {
            return url.starts_with("redis+cluster://") || url.starts_with("rediss+cluster://");
        }
        false
    }

    /// 规整连接 URL(对齐 `get_connection_url`:`+cluster` 前缀剥除)。
    pub fn get_connection_url(&self) -> Option<String> {
        let url = self.url.as_ref()?;
        if let Some(rest) = url.strip_prefix("redis+cluster://") {
            return Some(format!("redis://{rest}"));
        }
        if let Some(rest) = url.strip_prefix("rediss+cluster://") {
            return Some(format!("rediss://{rest}"));
        }
        Some(url.clone())
    }
}

/// Redis checkpointer 配置(对齐 `RedisCheckpointerConfig`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RedisCheckpointerConfig {
    pub connection: RedisConnectionConfig,
    #[serde(default)]
    pub ttl: Option<RedisTTLConfig>,
}

impl RedisCheckpointerConfig {
    /// 完整校验:URL 前缀 + 必填连接方式。
    pub fn validate(&self) -> Result<(), CheckpointerError> {
        self.connection.validate_url()?;
        self.connection.validate_present()
    }

    /// 从 JSON 映射解析(对齐 `model_validate`),失败包装为统一错误。
    pub fn from_json(value: &serde_json::Value) -> Result<Self, CheckpointerError> {
        let obj = value.as_object().ok_or_else(|| {
            CheckpointerError(
                "Invalid Redis checkpointer configuration: configuration must be an object. \
                 Configuration must have a 'connection' key with either 'redis_client' or 'url'. \
                 Optional 'ttl' key for TTL configuration."
                    .to_string(),
            )
        })?;
        let conn_value = obj.get("connection").ok_or_else(|| {
            CheckpointerError(
                "Invalid Redis checkpointer configuration: missing 'connection'. \
                 Configuration must have a 'connection' key with either 'redis_client' or 'url'. \
                 Optional 'ttl' key for TTL configuration."
                    .to_string(),
            )
        })?;
        let connection = serde_json::from_value::<RedisConnectionConfig>(conn_value.clone())
            .map_err(|e| {
                CheckpointerError(format!(
                    "Invalid Redis checkpointer configuration: {e}. \
                     Configuration must have a 'connection' key with either 'redis_client' or 'url'. \
                     Optional 'ttl' key for TTL configuration."
                ))
            })?;
        let ttl = match obj.get("ttl") {
            Some(v) if v.is_null() => None,
            Some(v) => Some(RedisTTLConfig::from_json(v)?),
            None => None,
        };
        Ok(Self { connection, ttl })
    }
}

// ---------------------------------------------------------------------------
// RedisStore 抽象(唯一 Redis 访问边界;对齐 extensions/store/kv/redis_store.py)
// ---------------------------------------------------------------------------

/// Redis 值类型:`str` UTF-8 编码 / `bytes` 原样。
#[derive(Debug, Clone, PartialEq)]
pub enum RedisValue {
    Str(String),
    Bytes(Vec<u8>),
}

/// pipeline 操作序列(对齐 redis-py pipeline 链式入队 + execute)。
#[derive(Debug, Clone, Default)]
pub struct RedisPipeline {
    ops: Vec<PipelineOp>,
}

/// 一次 pipeline 命令。
#[derive(Debug, Clone)]
pub enum PipelineOp {
    Set {
        key: String,
        value: RedisValue,
        ttl_seconds: Option<i64>,
    },
    Get {
        key: String,
    },
    Exists {
        key: String,
    },
}

impl RedisPipeline {
    /// 空 pipeline。
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    /// 入队 SET(key, value[, ex])。
    pub fn set(
        mut self,
        key: impl Into<String>,
        value: RedisValue,
        ttl_seconds: Option<i64>,
    ) -> Self {
        self.ops.push(PipelineOp::Set {
            key: key.into(),
            value,
            ttl_seconds,
        });
        self
    }

    /// 入队 GET。
    pub fn get(mut self, key: impl Into<String>) -> Self {
        self.ops.push(PipelineOp::Get { key: key.into() });
        self
    }

    /// 入队 EXISTS。
    pub fn exists(mut self, key: impl Into<String>) -> Self {
        self.ops.push(PipelineOp::Exists { key: key.into() });
        self
    }

    /// 已入队命令数。
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// 是否空。
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// 取命令列表(供执行方遍历)。
    pub fn ops(&self) -> &[PipelineOp] {
        &self.ops
    }
}

/// pipeline 执行结果(与入队顺序一致;缺失返回 None)。
pub type PipelineResult = Vec<Option<RedisValue>>;

/// RedisStore Seam(Service Definition):checkpointer 唯一的 Redis 访问边界。
///
/// 对齐 redis_store.py 的全部公开接口。实现方提供真实 Redis 或测试后端。
pub trait RedisStore: Seam {
    /// SET 键值;异常 re-raise。
    fn set(&self, key: &str, value: RedisValue) -> Result<(), CheckpointerError>;

    /// SET NX(key, value[, ex]);返回是否设置成功。
    fn exclusive_set(
        &self,
        key: &str,
        value: RedisValue,
        expiry_seconds: Option<i64>,
    ) -> Result<bool, CheckpointerError>;

    /// GET;缺失返回 None。
    fn get(&self, key: &str) -> Result<Option<RedisValue>, CheckpointerError>;

    /// EXISTS。
    fn exists(&self, key: &str) -> Result<bool, CheckpointerError>;

    /// DEL;删不存在的 key 返回 0,不算错误。
    fn delete(&self, key: &str) -> Result<i64, CheckpointerError>;

    /// SCAN MATCH "{prefix}*" 迭代 + 逐个 GET;返回 key→value,空则 {}。
    fn get_by_prefix(&self, prefix: &str) -> Result<Vec<(String, RedisValue)>, CheckpointerError>;

    /// SCAN MATCH "{prefix}*" 收集,按 batch_size 分批 DEL(batch_size≤0 一次全删)。
    fn delete_by_prefix(&self, prefix: &str, batch_size: i64) -> Result<i64, CheckpointerError>;

    /// MGET(失败回退逐个 GET;空入参返回空列表)。
    fn mget(&self, keys: &[String]) -> Result<Vec<Option<RedisValue>>, CheckpointerError>;

    /// 批量 DEL,按 batch_size 分块;返回实际删除数。
    fn batch_delete(&self, keys: &[String], batch_size: i64) -> Result<i64, CheckpointerError>;

    /// EXPIRE 批量刷新;keys 空或 ttl≤0 直接返回;异常静默吞掉。
    fn refresh_ttl(&self, keys: &[String], ttl_seconds: i64);

    /// pipeline 链式命令 + execute;结果与入队顺序一致。
    fn pipeline(&self, pipeline: &RedisPipeline) -> Result<PipelineResult, CheckpointerError>;
}

// ---------------------------------------------------------------------------
// Checkpointer / Storage / Provider 抽象(对齐 core/session/checkpointer/base.py)
// ---------------------------------------------------------------------------

/// 会话视图:checkpointer 钩子所需的会话属性(对齐 BaseSession + 子类)。
pub trait SessionView: Send + Sync {
    /// session id。
    fn session_id(&self) -> String;
    /// workflow id(None 表示非 workflow 会话)。
    fn workflow_id(&self) -> Option<String>;
    /// agent id(agent 会话)。
    fn agent_id(&self) -> Option<String>;
    /// 团队/组 id(运行期契约;agent-team 会话)。
    fn group_id(&self) -> Option<String>;
    /// 配置环境变量读取(对齐 Config.get_env)。
    fn get_env(&self, key: &str, default: bool) -> bool;
}

/// Checkpointer Seam(Service Definition):生命周期钩子编排。
///
/// 对齐 `Checkpointer` 抽象接口(redis 实现签名:release 带可选 agent_id)。
pub trait Checkpointer: Seam {
    /// agent 执行前:恢复 agent 状态 + 注入交互输入。
    fn pre_agent_execute(
        &self,
        session: &dyn SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError>;

    /// agent 团队执行前:恢复团队状态 + 注入交互输入(全局)。
    fn pre_agent_team_execute(
        &self,
        session: &dyn SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError>;

    /// agent 中断:保存 agent 状态。
    fn interrupt_agent_execute(&self, session: &dyn SessionView) -> Result<(), CheckpointerError>;

    /// agent 执行后:保存 agent 状态。
    fn post_agent_execute(&self, session: &dyn SessionView) -> Result<(), CheckpointerError>;

    /// agent 团队执行后:保存团队状态。
    fn post_agent_team_execute(&self, session: &dyn SessionView) -> Result<(), CheckpointerError>;

    /// workflow 执行前:交互输入恢复 / 非交互输入存在状态时强制删除或显式报错。
    fn pre_workflow_execute(
        &self,
        session: &dyn SessionView,
        inputs: Option<serde_json::Value>,
    ) -> Result<(), CheckpointerError>;

    /// workflow 执行后:异常保存+重抛 / 中断保存 / 完成清除。
    fn post_workflow_execute(
        &self,
        session: &dyn SessionView,
        result: &serde_json::Value,
        exception: Option<&str>,
    ) -> Result<(), CheckpointerError>;

    /// 会话是否存在(前缀扫描)。
    fn session_exists(&self, session_id: &str) -> Result<bool, CheckpointerError>;

    /// 释放会话资源(可选按 agent 释放)。
    fn release(&self, session_id: &str, agent_id: Option<&str>) -> Result<(), CheckpointerError>;

    /// 图状态存储(workflow-graph)。
    fn graph_store(&self) -> Option<serde_json::Value>;
}

/// CheckpointerProvider Seam(Service Definition):配置 → Checkpointer 工厂。
pub trait CheckpointerProvider: Seam {
    /// 从配置创建 Checkpointer(对齐 `CheckpointerFactory.register("redis")`)。
    fn create(&self, conf: &serde_json::Value) -> Result<Box<dyn Checkpointer>, CheckpointerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_key_joins_with_colons() {
        assert_eq!(build_key(&[]), "");
        assert_eq!(build_key(&["a", "b", "c"]), "a:b:c");
    }

    #[test]
    fn key_with_namespace_order_is_fixed() {
        let key = build_key_with_namespace("s1", "agent", "agent-1", &["agent_state_blobs"]);
        assert_eq!(key, "s1:agent:agent-1:agent_state_blobs");
        let graph = build_key_with_namespace(
            "s1",
            WORKFLOW_NAMESPACE_GRAPH,
            "wf",
            &["checkpoint_data_type"],
        );
        assert_eq!(graph, "s1:workflow-graph:wf:checkpoint_data_type");
    }

    #[test]
    fn ttl_conversion_truncates() {
        assert_eq!(ttl_seconds_from_minutes(Some(5.0)), Some(300));
        assert_eq!(ttl_seconds_from_minutes(Some(0.5)), Some(30));
        assert_eq!(ttl_seconds_from_minutes(None), None);
        // 浮点 5 分钟 → 300 秒(int 截断)。
        assert_eq!(ttl_seconds_from_minutes(Some(5.999)), Some(359));
    }

    #[test]
    fn connection_config_validates_url_prefix() {
        let ok = RedisConnectionConfig {
            url: Some("redis://localhost:6379".to_string()),
            ..Default::default()
        };
        assert!(ok.validate_url().is_ok());

        let bad = RedisConnectionConfig {
            url: Some("http://localhost:6379".to_string()),
            ..Default::default()
        };
        let err = bad.validate_url().expect_err("bad url");
        assert!(err.0.contains("Invalid Redis URL format"));
    }

    #[test]
    fn connection_config_requires_url() {
        let empty = RedisConnectionConfig::default();
        let err = empty.validate_present().expect_err("missing url");
        assert!(err.0.contains("Either 'redis_client' or 'url'"));
    }

    #[test]
    fn cluster_mode_and_url_normalization() {
        let explicit = RedisConnectionConfig {
            url: Some("redis://h:7000".to_string()),
            cluster_mode: Some(true),
            ..Default::default()
        };
        assert!(explicit.is_cluster_mode());

        let scheme = RedisConnectionConfig {
            url: Some("redis+cluster://h:7000".to_string()),
            cluster_mode: None,
            ..Default::default()
        };
        assert!(scheme.is_cluster_mode());
        assert_eq!(
            scheme.get_connection_url().as_deref(),
            Some("redis://h:7000")
        );

        let plain = RedisConnectionConfig {
            url: Some("redis://h:6379".to_string()),
            cluster_mode: None,
            ..Default::default()
        };
        assert!(!plain.is_cluster_mode());
        assert_eq!(
            plain.get_connection_url().as_deref(),
            Some("redis://h:6379")
        );
    }

    #[test]
    fn full_config_parses_and_validates() {
        let conf = serde_json::json!({
            "connection": {"url": "redis://localhost:6379"},
            "ttl": {"default_ttl": 5.0, "refresh_on_read": true}
        });
        let config = RedisCheckpointerConfig::from_json(&conf).expect("parse");
        assert!(config.validate().is_ok());
        let ttl = config.ttl.expect("ttl");
        assert_eq!(ttl.default_ttl, Some(5.0));
        assert!(ttl.refresh_on_read);
        assert_eq!(ttl_seconds_from_minutes(ttl.default_ttl), Some(300));
    }

    #[test]
    fn full_config_rejects_missing_connection() {
        let conf = serde_json::json!({"ttl": {}});
        let err = RedisCheckpointerConfig::from_json(&conf).expect_err("missing connection");
        assert!(err.0.contains("missing 'connection'"));
    }

    #[test]
    fn pipeline_builds_op_sequence() {
        let pipeline = RedisPipeline::new()
            .set("k1", RedisValue::Str("v1".to_string()), Some(300))
            .get("k2")
            .exists("k3");
        assert_eq!(pipeline.len(), 3);
        assert!(!pipeline.is_empty());
    }
}
