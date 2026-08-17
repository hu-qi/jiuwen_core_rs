//! model-allocator seam:团队模型池分配器(对齐 openjiuwen/agent_teams/models/allocator.py)。
//!
//! 当 `TeamSpec.model_pool` 非空时,分配器按策略把池条目分发给 leader /
//! teammate,使并发调用摊到多个端点而不是压满单个端点。四种策略:
//! - `round_robin`:池序线性轮转,无视 `model_name`(池内条目可互换);
//! - `by_model_name`:按 `model_name` 分组(保插入序),组内轮转;
//! - `router`:单端点路由,`model_name` 唯一映射,无 hint 返回首项(确定性);
//! - `intelli_router`:`router` 语义 + 构造校验(provider / deployments)。
//!
//! 身份模型:每次分配引用 `(model_name, group_index)` —— 条目在池中同名
//! 组内的位置。DB 只持久化该轻量引用,活配置(凭证 / 端点)由会话内池经
//! `resolve_member_model` 重新水合。
//!
//! 契约零实现:具体分配器与策略工厂由插件提供(如 ah-plugins-model-allocator)。

use serde_json::Value;

use crate::seam::Seam;

/// 模型池条目:一个 LLM 端点 + provider + 自由 metadata。
///
/// `metadata` 为自由 JSON(如 `client.intelli_router_deployments`)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelPoolEntry {
    pub model_name: String,
    pub api_provider: String,
    pub metadata: Value,
}

impl ModelPoolEntry {
    /// 便捷构造(metadata 缺省为空对象)。
    pub fn new(model_name: impl Into<String>, api_provider: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            api_provider: api_provider.into(),
            metadata: serde_json::json!({}),
        }
    }

    /// 带自由 metadata 构造。
    pub fn with_metadata(
        model_name: impl Into<String>,
        api_provider: impl Into<String>,
        metadata: Value,
    ) -> Self {
        Self {
            model_name: model_name.into(),
            api_provider: api_provider.into(),
            metadata,
        }
    }
}

/// 一次分配结果:选中的池条目 + 组内位置。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Allocation {
    pub entry: ModelPoolEntry,
    pub group_index: usize,
}

impl Allocation {
    /// 轻量 DB 引用 `(model_name, group_index)`(对齐 `Allocation.to_db_ref`)。
    pub fn to_db_ref(&self) -> (String, usize) {
        (self.entry.model_name.clone(), self.group_index)
    }
}

/// 分配策略(serde snake_case:round_robin / by_model_name / router / intelli_router)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocatorStrategy {
    RoundRobin,
    ByModelName,
    Router,
    IntelliRouter,
}

/// 分配器构造错误(池形状违规:空池 / 重名 / provider 或 deployments 不合法)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelAllocError(pub String);

impl core::fmt::Display for ModelAllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ModelAllocError {}

/// 模型分配器 Seam(Service Definition):每次调用分配一个池条目。
///
/// 返回 `None` 表示"无可用条目"——调用方回退到成员自己的 per-agent 模型。
/// 实现需暴露 `state_dict` / `load_state_dict` 以便轮转计数跨全量重启恢复;
/// 池本身在 `TeamSpec` 上、恢复时重建,只有易变计数与池 digest 随会话保存。
/// `load_state_dict` 在持久化 digest 与当前池不匹配时自动归零。
pub trait ModelAllocator: Seam {
    /// 返回下一次分配;`model_name` 为可选提示(按名策略需要,轮转策略忽略)。
    fn allocate(&self, model_name: Option<&str>) -> Option<Allocation>;

    /// 快照轮转计数 + 池 digest(可 JSON 往返)。
    fn state_dict(&self) -> Value;

    /// 恢复计数;池 digest 不匹配时归零(容忍缺键 / 未知键)。
    fn load_state_dict(&self, state: &Value);
}

/// 分配器策略工厂 Seam:按策略构造具体分配器。
pub trait ModelAllocatorFactory: Seam {
    /// 构造对应策略的分配器;池形状违规返回 `ModelAllocError`。
    fn build_allocator(
        &self,
        pool: &[ModelPoolEntry],
        strategy: AllocatorStrategy,
    ) -> Result<Box<dyn ModelAllocator>, ModelAllocError>;
}
