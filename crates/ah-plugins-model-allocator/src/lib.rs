//! # ah-plugins-model-allocator
//!
//! 真实团队模型分配器(对齐 openjiuwen/agent_teams/models/allocator.py):
//! - `RoundRobinModelAllocator`:池序线性轮转,无视 `model_name`;
//! - `ByModelNameAllocator`:按 `model_name` 分组(保插入序)、组内轮转;
//! - `RouterAllocator`:单端点路由,`model_name` 唯一映射,无 hint 返回首项;
//! - `IntelliRouterAllocator`:`router` 语义 + 构造校验(provider / deployments);
//! - `build_allocator` 策略工厂 + `resolve_member_model` 纯位置查找;
//! - `ModelAllocatorPlugin` 把策略工厂注册到 `model-allocator` seam。
//!
//! 可靠性分层:前三者把成员摊到多个端点(可靠性归 allocator);
//! `intelli_router` 把多端点整个下沉给客户端 router(可靠性归 client)。
//! 两者二选一,叠加即重复做负载均衡。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use ah_contracts::keys::MODEL_ALLOCATOR;
use ah_contracts::model_allocator::{
    Allocation, AllocatorStrategy, ModelAllocError, ModelAllocator, ModelAllocatorFactory,
    ModelPoolEntry,
};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

/// 池结构 digest:按序对每个 entry 拼接 (model_name, api_provider),FNV-1a 64。
///
/// 重排 / 增删条目会改变 digest(触发 `load_state_dict` 归零);
/// 凭证或 metadata 变更不改变(原地刷新不失效计数)。
fn pool_digest(pool: &[ModelPoolEntry]) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for entry in pool {
        for &b in entry.model_name.as_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0x00;
        hash = hash.wrapping_mul(PRIME);
        for &b in entry.api_provider.as_bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// 池序线性轮转分配器:无视 `model_name`,按池序逐个分配、到尾回绕。
#[derive(Debug)]
pub struct RoundRobinModelAllocator {
    pool: Vec<ModelPoolEntry>,
    /// 同名组的池内下标(保插入序);组序按首见顺序。
    groups: Vec<Vec<usize>>,
    name_to_group: HashMap<String, usize>,
    digest: String,
    index: Mutex<usize>,
}

impl Seam for RoundRobinModelAllocator {}

impl RoundRobinModelAllocator {
    /// 用池条目构造轮转分配器(空池合法:allocate 恒为 None)。
    pub fn new(pool: &[ModelPoolEntry]) -> Self {
        let pool = pool.to_vec();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut name_to_group: HashMap<String, usize> = HashMap::new();
        for (i, entry) in pool.iter().enumerate() {
            let group = *name_to_group
                .entry(entry.model_name.clone())
                .or_insert_with(|| {
                    groups.push(Vec::new());
                    groups.len() - 1
                });
            groups[group].push(i);
        }
        let digest = pool_digest(&pool);
        Self {
            pool,
            groups,
            name_to_group,
            digest,
            index: Mutex::new(0),
        }
    }
}

impl ModelAllocator for RoundRobinModelAllocator {
    fn allocate(&self, model_name: Option<&str>) -> Option<Allocation> {
        let _ = model_name; // round-robin 无视 model_name
        if self.pool.is_empty() {
            return None;
        }
        let mut index = self.index.lock().unwrap();
        let pos = *index % self.pool.len();
        *index += 1;
        let entry = self.pool[pos].clone();
        let group = &self.groups[self.name_to_group[&entry.model_name]];
        let group_index = group
            .iter()
            .position(|&i| i == pos)
            .expect("entry always belongs to its name group");
        Some(Allocation { entry, group_index })
    }

    fn state_dict(&self) -> Value {
        json!({
            "index": *self.index.lock().unwrap(),
            "pool_digest": self.digest,
        })
    }

    fn load_state_dict(&self, state: &Value) {
        let mut index = self.index.lock().unwrap();
        let persisted = state.get("pool_digest").and_then(Value::as_str);
        if persisted != Some(self.digest.as_str()) {
            *index = 0;
            return;
        }
        *index = state.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    }
}

/// 按 `model_name` 分组、组内轮转的分配器。
#[derive(Debug)]
pub struct ByModelNameAllocator {
    pool: Vec<ModelPoolEntry>,
    /// 组名(保插入序)。
    names: Vec<String>,
    /// 每组的池内下标(保插入序)。
    groups: Vec<Vec<usize>>,
    name_to_group: HashMap<String, usize>,
    digest: String,
    /// 每组的轮转计数。
    counters: Mutex<Vec<usize>>,
}

impl Seam for ByModelNameAllocator {}

impl ByModelNameAllocator {
    /// 按 `model_name` 分区构造(空池合法:allocate 恒为 None)。
    pub fn new(pool: &[ModelPoolEntry]) -> Self {
        let pool = pool.to_vec();
        let mut names: Vec<String> = Vec::new();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut name_to_group: HashMap<String, usize> = HashMap::new();
        for (i, entry) in pool.iter().enumerate() {
            let group = *name_to_group
                .entry(entry.model_name.clone())
                .or_insert_with(|| {
                    names.push(entry.model_name.clone());
                    groups.push(Vec::new());
                    groups.len() - 1
                });
            groups[group].push(i);
        }
        let digest = pool_digest(&pool);
        let counters = Mutex::new(vec![0; groups.len()]);
        Self {
            pool,
            names,
            groups,
            name_to_group,
            digest,
            counters,
        }
    }
}

impl ModelAllocator for ByModelNameAllocator {
    fn allocate(&self, model_name: Option<&str>) -> Option<Allocation> {
        let name = model_name?;
        let group_idx = *self.name_to_group.get(name)?;
        let group = &self.groups[group_idx];
        let mut counters = self.counters.lock().unwrap();
        let pos_in_group = counters[group_idx] % group.len();
        counters[group_idx] += 1;
        let pos = group[pos_in_group];
        let entry = self.pool[pos].clone();
        Some(Allocation {
            entry,
            group_index: pos_in_group,
        })
    }

    fn state_dict(&self) -> Value {
        let counters = self.counters.lock().unwrap();
        let list: Vec<Value> = self
            .names
            .iter()
            .zip(counters.iter())
            .map(|(name, index)| json!({ "model_name": name, "index": index }))
            .collect();
        json!({ "counters": list, "pool_digest": self.digest })
    }

    fn load_state_dict(&self, state: &Value) {
        let mut counters = self.counters.lock().unwrap();
        let persisted = state.get("pool_digest").and_then(Value::as_str);
        if persisted != Some(self.digest.as_str()) {
            for c in counters.iter_mut() {
                *c = 0;
            }
            return;
        }
        match state.get("counters") {
            Some(Value::Array(records)) => {
                for record in records {
                    let Some(obj) = record.as_object() else {
                        continue;
                    };
                    let Some(Value::String(name)) = obj.get("model_name") else {
                        continue;
                    };
                    let Some(&group_idx) = self.name_to_group.get(name) else {
                        continue;
                    };
                    counters[group_idx] =
                        obj.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                }
            }
            _ => {
                // legacy 格式:inner_indexes dict(model_name -> int)。
                if let Some(Value::Object(legacy)) = state.get("inner_indexes") {
                    for (name, raw) in legacy {
                        let Some(&group_idx) = self.name_to_group.get(name) else {
                            continue;
                        };
                        counters[group_idx] = raw.as_u64().unwrap_or(0) as usize;
                    }
                }
            }
        }
    }
}

/// 单端点路由分配器:`model_name` 唯一映射,无 hint 返回首项,无轮转计数。
#[derive(Debug)]
pub struct RouterAllocator {
    pool: Vec<ModelPoolEntry>,
    /// model_name → 池内下标。
    by_name: HashMap<String, usize>,
    digest: String,
}

impl Seam for RouterAllocator {}

impl RouterAllocator {
    /// 构造;池空或 `model_name` 重复 → 显式 Err。
    pub fn new(pool: &[ModelPoolEntry]) -> Result<Self, ModelAllocError> {
        if pool.is_empty() {
            return Err(ModelAllocError(
                "RouterAllocator requires a non-empty pool".into(),
            ));
        }
        let mut seen: HashMap<&str, ()> = HashMap::new();
        let mut duplicates: Vec<&str> = Vec::new();
        for entry in pool {
            if seen.insert(entry.model_name.as_str(), ()).is_some() {
                duplicates.push(entry.model_name.as_str());
            }
        }
        if !duplicates.is_empty() {
            duplicates.sort_unstable();
            duplicates.dedup();
            return Err(ModelAllocError(format!(
                "RouterAllocator pool must have unique model_names; duplicates: {duplicates:?}"
            )));
        }
        let pool = pool.to_vec();
        let by_name = pool
            .iter()
            .enumerate()
            .map(|(i, e)| (e.model_name.clone(), i))
            .collect();
        let digest = pool_digest(&pool);
        Ok(Self {
            pool,
            by_name,
            digest,
        })
    }
}

impl ModelAllocator for RouterAllocator {
    fn allocate(&self, model_name: Option<&str>) -> Option<Allocation> {
        let pos = match model_name {
            None => 0,
            Some(name) => *self.by_name.get(name)?,
        };
        Some(Allocation {
            entry: self.pool[pos].clone(),
            group_index: 0,
        })
    }

    fn state_dict(&self) -> Value {
        json!({ "pool_digest": self.digest })
    }

    fn load_state_dict(&self, _state: &Value) {
        // 路由分配无轮转计数:digest 变更也无计数可归零,恢复为 no-op。
    }
}

/// IntelliRouter 池分配器:Router 语义 + 构造校验。
///
/// 校验在构造期把手写池的配置错误变成 `build` 时的显式错误,
/// 而不是在首个请求才以空凭证静默失败。
#[derive(Debug)]
pub struct IntelliRouterAllocator {
    inner: RouterAllocator,
}

impl Seam for IntelliRouterAllocator {}

impl IntelliRouterAllocator {
    /// 构造;继承 router 校验(空池 / 重名),再校验 provider 与 deployments。
    pub fn new(pool: &[ModelPoolEntry]) -> Result<Self, ModelAllocError> {
        let inner = RouterAllocator::new(pool)?;
        let mut mismatched: Vec<String> = pool
            .iter()
            .filter(|entry| entry.api_provider != "intelli_router")
            .map(|entry| format!("{} declares {}", entry.model_name, entry.api_provider))
            .collect();
        if !mismatched.is_empty() {
            mismatched.sort();
            return Err(ModelAllocError(format!(
                "model_pool_strategy='intelli_router' requires every entry to declare api_provider='intelli_router'; offending entries: {mismatched:?}"
            )));
        }
        let mut empty: Vec<String> = pool
            .iter()
            .filter(|entry| !has_deployments(entry))
            .map(|entry| entry.model_name.clone())
            .collect();
        if !empty.is_empty() {
            empty.sort();
            return Err(ModelAllocError(format!(
                "model_pool_strategy='intelli_router' requires every entry to carry a non-empty metadata.client.intelli_router_deployments list; missing for: {empty:?}"
            )));
        }
        Ok(Self { inner })
    }
}

impl ModelAllocator for IntelliRouterAllocator {
    fn allocate(&self, model_name: Option<&str>) -> Option<Allocation> {
        self.inner.allocate(model_name)
    }

    fn state_dict(&self) -> Value {
        self.inner.state_dict()
    }

    fn load_state_dict(&self, state: &Value) {
        self.inner.load_state_dict(state)
    }
}

/// entry 是否携带非空 `metadata.client.intelli_router_deployments` 列表。
fn has_deployments(entry: &ModelPoolEntry) -> bool {
    entry
        .metadata
        .get("client")
        .and_then(Value::as_object)
        .and_then(|client| client.get("intelli_router_deployments"))
        .and_then(Value::as_array)
        .is_some_and(|deployments| !deployments.is_empty())
}

/// 按策略构造分配器。
///
/// `round_robin` / `by_model_name` 允许空池(allocate 恒为 None);
/// `router` / `intelli_router` 空池或形状违规 → 显式 Err。
pub fn build_allocator(
    pool: &[ModelPoolEntry],
    strategy: AllocatorStrategy,
) -> Result<Box<dyn ModelAllocator>, ModelAllocError> {
    match strategy {
        AllocatorStrategy::RoundRobin => Ok(Box::new(RoundRobinModelAllocator::new(pool))),
        AllocatorStrategy::ByModelName => Ok(Box::new(ByModelNameAllocator::new(pool))),
        AllocatorStrategy::Router => Ok(Box::new(RouterAllocator::new(pool)?)),
        AllocatorStrategy::IntelliRouter => Ok(Box::new(IntelliRouterAllocator::new(pool)?)),
    }
}

/// 纯位置查找:把持久化的 `(model_name, model_index)` 引用解析为池中
/// entry 的下标。不触碰分配器、不推进任何轮转计数。
///
/// - 组存在且 index 在范围 → 返回该 entry 的池下标;
/// - 组存在但 index 越界(组缩小)→ 返回组内 index 0(确定性回退);
/// - 组不存在 / 池空 / 名缺失或为空 → None(调用方回退 per-agent 模型)。
pub fn resolve_member_model(
    pool: &[ModelPoolEntry],
    model_name: Option<&str>,
    model_index: Option<usize>,
) -> Option<usize> {
    let name = model_name.filter(|n| !n.is_empty())?;
    if pool.is_empty() {
        return None;
    }
    let group: Vec<usize> = pool
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.model_name == name)
        .map(|(i, _)| i)
        .collect();
    if group.is_empty() {
        return None;
    }
    match model_index {
        Some(idx) if idx < group.len() => Some(group[idx]),
        _ => Some(group[0]),
    }
}

/// 策略工厂实现(`model-allocator` seam 的服务提供者)。
pub struct ModelAllocatorBuilder;

impl Seam for ModelAllocatorBuilder {}

impl ModelAllocatorFactory for ModelAllocatorBuilder {
    fn build_allocator(
        &self,
        pool: &[ModelPoolEntry],
        strategy: AllocatorStrategy,
    ) -> Result<Box<dyn ModelAllocator>, ModelAllocError> {
        build_allocator(pool, strategy)
    }
}

/// model-allocator 插件:注册 `model-allocator` seam(策略工厂)。
pub struct ModelAllocatorPlugin;

impl Plugin for ModelAllocatorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-model-allocator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MODEL_ALLOCATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let factory: Arc<dyn ModelAllocatorFactory> = Arc::new(ModelAllocatorBuilder);
        Ok(vec![ctx.register(MODEL_ALLOCATOR, factory)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::MODEL_ALLOCATOR;
    use ah_hub::plugin::DynPlugin;

    fn entry(name: &str, provider: &str) -> ModelPoolEntry {
        ModelPoolEntry::new(name, provider)
    }

    fn ir_entry(name: &str, deployments: &[&str]) -> ModelPoolEntry {
        let list: Vec<Value> = deployments
            .iter()
            .map(|d| json!({ "api_base": d }))
            .collect();
        ModelPoolEntry::with_metadata(
            name,
            "intelli_router",
            json!({ "client": { "intelli_router_deployments": list } }),
        )
    }

    /// state_dict → JSON 字符串 → 解析回 Value(模拟会话持久层往返)。
    fn roundtrip(state: &Value) -> Value {
        serde_json::from_str(&serde_json::to_string(state).unwrap()).unwrap()
    }

    #[test]
    fn round_robin_rotates_in_pool_order_ignoring_hint() {
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-c", "openai"),
        ];
        let allocator = RoundRobinModelAllocator::new(&pool);
        assert_eq!(
            allocator
                .allocate(Some("ignored"))
                .unwrap()
                .entry
                .model_name,
            "m-a"
        );
        assert_eq!(allocator.allocate(None).unwrap().entry.model_name, "m-b");
        assert_eq!(
            allocator
                .allocate(Some("ignored"))
                .unwrap()
                .entry
                .model_name,
            "m-c"
        );
        // 到尾回绕。
        assert_eq!(allocator.allocate(None).unwrap().entry.model_name, "m-a");
    }

    #[test]
    fn round_robin_empty_pool_returns_none() {
        let allocator = RoundRobinModelAllocator::new(&[]);
        assert_eq!(allocator.allocate(None), None);
        assert_eq!(allocator.allocate(Some("m-a")), None);
    }

    #[test]
    fn round_robin_group_index_within_same_name_group() {
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-a", "azure"),
        ];
        let allocator = RoundRobinModelAllocator::new(&pool);
        let first = allocator.allocate(None).unwrap();
        assert_eq!(first.to_db_ref(), ("m-a".to_string(), 0));
        let second = allocator.allocate(None).unwrap();
        assert_eq!(second.to_db_ref(), ("m-b".to_string(), 0));
        let third = allocator.allocate(None).unwrap();
        assert_eq!(third.to_db_ref(), ("m-a".to_string(), 1));
        assert_eq!(third.entry.api_provider, "azure");
    }

    #[test]
    fn round_robin_state_dict_resume_and_digest_mismatch_reset() {
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-c", "openai"),
        ];
        let first = RoundRobinModelAllocator::new(&pool);
        assert_eq!(first.allocate(None).unwrap().entry.model_name, "m-a");
        assert_eq!(first.allocate(None).unwrap().entry.model_name, "m-b");
        let state = first.state_dict();
        // 同一池恢复:续上轮转(下一个是 m-c)。
        let resumed = RoundRobinModelAllocator::new(&pool);
        resumed.load_state_dict(&roundtrip(&state));
        assert_eq!(resumed.allocate(None).unwrap().entry.model_name, "m-c");
        // digest 不匹配(重排池)→ 归零,下一个是重排后的首项。
        let reordered = [
            entry("m-c", "openai"),
            entry("m-a", "openai"),
            entry("m-b", "openai"),
        ];
        let other = RoundRobinModelAllocator::new(&reordered);
        other.load_state_dict(&roundtrip(&state));
        assert_eq!(other.allocate(None).unwrap().entry.model_name, "m-c");
    }

    #[test]
    fn round_robin_load_tolerates_missing_and_bad_fields() {
        let pool = [entry("m-a", "openai"), entry("m-b", "openai")];
        let allocator = RoundRobinModelAllocator::new(&pool);
        allocator.allocate(None);
        // 缺 pool_digest → 视为不匹配 → 归零。
        allocator.load_state_dict(&json!({}));
        assert_eq!(allocator.allocate(None).unwrap().entry.model_name, "m-a");
        // index 非数字 → 防御为 0。
        let digest = allocator.state_dict()["pool_digest"].clone();
        allocator.load_state_dict(&json!({ "index": "oops", "pool_digest": digest }));
        assert_eq!(allocator.allocate(None).unwrap().entry.model_name, "m-a");
    }

    #[test]
    fn by_name_rotates_within_group_preserving_insertion_order() {
        let pool = [
            entry("m-x", "openai"),
            entry("m-y", "openai"),
            entry("m-x", "azure"),
            entry("m-x", "anthropic"),
        ];
        let allocator = ByModelNameAllocator::new(&pool);
        let a = allocator.allocate(Some("m-x")).unwrap();
        assert_eq!(a.entry.api_provider, "openai");
        assert_eq!(a.group_index, 0);
        let b = allocator.allocate(Some("m-x")).unwrap();
        assert_eq!(b.entry.api_provider, "azure");
        assert_eq!(b.group_index, 1);
        let c = allocator.allocate(Some("m-x")).unwrap();
        assert_eq!(c.entry.api_provider, "anthropic");
        assert_eq!(c.group_index, 2);
        // 组内回绕。
        let d = allocator.allocate(Some("m-x")).unwrap();
        assert_eq!(d.entry.api_provider, "openai");
        assert_eq!(d.group_index, 0);
        // 独立组互不影响。
        let y = allocator.allocate(Some("m-y")).unwrap();
        assert_eq!(y.entry.model_name, "m-y");
        assert_eq!(y.group_index, 0);
    }

    #[test]
    fn by_name_unknown_or_missing_name_returns_none() {
        let pool = [entry("m-a", "openai")];
        let allocator = ByModelNameAllocator::new(&pool);
        assert_eq!(allocator.allocate(None), None);
        assert_eq!(allocator.allocate(Some("nope")), None);
        assert_eq!(allocator.allocate(Some("")), None);
    }

    #[test]
    fn by_name_state_dict_counters_list_roundtrip_with_dotted_names() {
        // 含 '.' 的模型名证明 counters 用 list(若用 dict,键含点号会被会话
        // 持久层解释为嵌套路径编码而重写键)。
        let pool = [
            entry("glm-5.1", "openai"),
            entry("glm-5.1", "azure"),
            entry("claude-3.5-sonnet", "anthropic"),
        ];
        let allocator = ByModelNameAllocator::new(&pool);
        allocator.allocate(Some("glm-5.1")); // counter glm-5.1 = 1
        let state = allocator.state_dict();
        let counters = state["counters"]
            .as_array()
            .expect("counters must be a list");
        assert_eq!(counters.len(), 2);
        let glm = counters
            .iter()
            .find(|r| r["model_name"] == "glm-5.1")
            .expect("glm-5.1 record");
        assert_eq!(glm["index"].as_u64(), Some(1));
        // JSON 往返后恢复到新实例:glm-5.1 组下一个是第二个条目。
        let resumed = ByModelNameAllocator::new(&pool);
        resumed.load_state_dict(&roundtrip(&state));
        let next = resumed.allocate(Some("glm-5.1")).unwrap();
        assert_eq!(next.entry.api_provider, "azure");
        assert_eq!(next.group_index, 1);
    }

    #[test]
    fn by_name_load_legacy_inner_indexes_dict_and_digest_mismatch() {
        // m-b 组有两个条目,legacy 计数 3 才能落到组内下标 1(3 % 2 = 1)。
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-b", "azure"),
        ];
        let digest = ByModelNameAllocator::new(&pool).state_dict()["pool_digest"].clone();
        let legacy = json!({
            "pool_digest": digest,
            "inner_indexes": { "m-b": 3, "unknown": 7 },
        });
        let allocator = ByModelNameAllocator::new(&pool);
        allocator.load_state_dict(&roundtrip(&legacy));
        // m-b 计数 3 → 3 % 2 = 1 → 组内第二个;未知名被忽略。
        assert_eq!(allocator.allocate(Some("m-b")).unwrap().group_index, 1);
        assert_eq!(allocator.allocate(Some("m-a")).unwrap().group_index, 0);
        // digest 不匹配 → 全组归零。
        let mismatched = json!({
            "pool_digest": "stale-digest",
            "inner_indexes": { "m-b": 3 },
        });
        let other = ByModelNameAllocator::new(&pool);
        other.load_state_dict(&mismatched);
        assert_eq!(other.allocate(Some("m-b")).unwrap().group_index, 0);
    }

    #[test]
    fn router_unique_mapping_and_default_first() {
        let pool = [entry("gpt-5", "openrouter"), entry("gpt-5.1", "openrouter")];
        let allocator = RouterAllocator::new(&pool).expect("router pool valid");
        // 无 hint → 首个声明名(leader 默认模型)。
        let d = allocator.allocate(None).unwrap();
        assert_eq!(d.to_db_ref(), ("gpt-5".to_string(), 0));
        // 按名唯一映射,group_index 恒 0,确定性(无轮转)。
        assert_eq!(
            allocator
                .allocate(Some("gpt-5.1"))
                .unwrap()
                .entry
                .model_name,
            "gpt-5.1"
        );
        assert_eq!(
            allocator
                .allocate(Some("gpt-5.1"))
                .unwrap()
                .entry
                .model_name,
            "gpt-5.1"
        );
        assert_eq!(
            allocator.allocate(Some("gpt-5")).unwrap().entry.model_name,
            "gpt-5"
        );
    }

    #[test]
    fn router_unknown_name_returns_none() {
        let pool = [entry("gpt-5", "openrouter")];
        let allocator = RouterAllocator::new(&pool).expect("router pool valid");
        assert_eq!(allocator.allocate(Some("nope")), None);
        assert_eq!(allocator.allocate(Some("")), None);
    }

    #[test]
    fn router_validation_rejects_empty_and_duplicate_names() {
        let empty: Vec<ModelPoolEntry> = Vec::new();
        let err = RouterAllocator::new(&empty).expect_err("empty pool must fail");
        assert!(err.0.contains("non-empty"), "{}", err.0);
        let dup = [
            entry("gpt-5", "openrouter"),
            entry("gpt-5", "openrouter"),
            entry("gpt-6", "openrouter"),
        ];
        let err = RouterAllocator::new(&dup).expect_err("duplicates must fail");
        assert!(err.0.contains("duplicates"), "{}", err.0);
        assert!(err.0.contains("gpt-5"), "{}", err.0);
        // 工厂路径同样显式报错。
        assert!(build_allocator(&dup, AllocatorStrategy::Router).is_err());
        assert!(build_allocator(&empty, AllocatorStrategy::Router).is_err());
    }

    #[test]
    fn intelli_router_rejects_wrong_provider() {
        let pool = [
            entry("m-a", "openai"),
            ir_entry("m-b", &["https://d1.example.com"]),
        ];
        let err = IntelliRouterAllocator::new(&pool).expect_err("wrong provider must fail");
        assert!(err.0.contains("m-a declares openai"), "{}", err.0);
        assert!(err.0.contains("intelli_router"), "{}", err.0);
    }

    #[test]
    fn intelli_router_rejects_missing_or_empty_deployments() {
        // metadata 缺 client / deployments。
        let missing = [ModelPoolEntry::with_metadata(
            "m-a",
            "intelli_router",
            json!({}),
        )];
        let err = IntelliRouterAllocator::new(&missing).expect_err("missing deployments must fail");
        assert!(err.0.contains("intelli_router_deployments"), "{}", err.0);
        // deployments 为空列表。
        let empty = [ir_entry("m-a", &[])];
        let err = IntelliRouterAllocator::new(&empty).expect_err("empty deployments must fail");
        assert!(err.0.contains("m-a"), "{}", err.0);
    }

    #[test]
    fn intelli_router_valid_pool_allocates_like_router() {
        let pool = [
            ir_entry("m-a", &["https://d1.example.com", "https://d2.example.com"]),
            ir_entry("m-b", &["https://d3.example.com"]),
        ];
        let allocator = IntelliRouterAllocator::new(&pool).expect("valid intelli pool");
        assert_eq!(
            allocator.allocate(None).unwrap().to_db_ref(),
            ("m-a".to_string(), 0)
        );
        assert_eq!(
            allocator.allocate(Some("m-b")).unwrap().to_db_ref(),
            ("m-b".to_string(), 0)
        );
        // state_dict 仅 pool_digest;load 为 no-op(无轮转计数)。
        let state = allocator.state_dict();
        assert!(state.get("pool_digest").is_some());
        assert!(state.get("index").is_none());
        allocator.load_state_dict(&json!({ "pool_digest": "stale" }));
        assert_eq!(
            allocator.allocate(Some("m-b")).unwrap().entry.model_name,
            "m-b"
        );
    }

    #[test]
    fn resolve_member_model_returns_pool_index_when_in_range() {
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-a", "azure"),
        ];
        assert_eq!(resolve_member_model(&pool, Some("m-a"), Some(0)), Some(0));
        assert_eq!(resolve_member_model(&pool, Some("m-a"), Some(1)), Some(2));
        assert_eq!(resolve_member_model(&pool, Some("m-b"), Some(0)), Some(1));
    }

    #[test]
    fn resolve_member_model_shrunk_group_falls_back_to_zero() {
        // 组 'm-a' 从 2 个缩到 1 个:越界 index 回退组内 0。
        let pool = [entry("m-a", "openai"), entry("m-b", "openai")];
        assert_eq!(resolve_member_model(&pool, Some("m-a"), Some(5)), Some(0));
        assert_eq!(resolve_member_model(&pool, Some("m-a"), None), Some(0));
    }

    #[test]
    fn resolve_member_model_missing_group_or_empty_pool_returns_none() {
        let pool = [entry("m-a", "openai")];
        assert_eq!(resolve_member_model(&pool, Some("nope"), Some(0)), None);
        assert_eq!(resolve_member_model(&pool, None, Some(0)), None);
        assert_eq!(resolve_member_model(&pool, Some(""), Some(0)), None);
        assert_eq!(resolve_member_model(&[], Some("m-a"), Some(0)), None);
    }

    #[test]
    fn build_allocator_factory_constructs_each_strategy() {
        let pool = [
            entry("m-a", "openai"),
            entry("m-b", "openai"),
            entry("m-a", "azure"),
        ];
        let rr = build_allocator(&pool, AllocatorStrategy::RoundRobin).expect("round robin");
        assert_eq!(rr.allocate(None).unwrap().entry.model_name, "m-a");
        let by_name =
            build_allocator(&pool, AllocatorStrategy::ByModelName).expect("by model name");
        assert_eq!(
            by_name.allocate(Some("m-b")).unwrap().entry.model_name,
            "m-b"
        );
        // router 要求 model_name 唯一,用独立池。
        let router_pool = [entry("m-a", "openai"), entry("m-b", "openai")];
        let router = build_allocator(&router_pool, AllocatorStrategy::Router).expect("router");
        assert_eq!(router.allocate(None).unwrap().entry.model_name, "m-a");
        let ir_pool = [ir_entry("m-a", &["https://d1.example.com"])];
        let ir =
            build_allocator(&ir_pool, AllocatorStrategy::IntelliRouter).expect("intelli router");
        assert_eq!(ir.allocate(None).unwrap().entry.model_name, "m-a");
        // 空池:轮转 / 按名合法(allocate None),router / intelli 显式 Err。
        let empty: Vec<ModelPoolEntry> = Vec::new();
        assert_eq!(
            build_allocator(&empty, AllocatorStrategy::RoundRobin)
                .unwrap()
                .allocate(None),
            None
        );
        assert_eq!(
            build_allocator(&empty, AllocatorStrategy::ByModelName)
                .unwrap()
                .allocate(None),
            None
        );
        assert!(build_allocator(&empty, AllocatorStrategy::Router).is_err());
        assert!(build_allocator(&empty, AllocatorStrategy::IntelliRouter).is_err());
    }

    #[test]
    fn plugin_registers_model_allocator_factory() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ModelAllocatorPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let factory = ctx
            .service::<dyn ModelAllocatorFactory>(&MODEL_ALLOCATOR)
            .expect("model-allocator seam");
        let pool = [entry("m-a", "openai"), entry("m-b", "openai")];
        let allocator = factory
            .build_allocator(&pool, AllocatorStrategy::RoundRobin)
            .expect("build via factory");
        assert_eq!(allocator.allocate(None).unwrap().entry.model_name, "m-a");
        // 注册可逆:drop effects 后 seam 消失。
        drop(effects);
        assert!(!ctx.has_service(&MODEL_ALLOCATOR));
    }
}
