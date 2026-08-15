//! graph memory seam:知识图谱记忆(对齐 Python memory/graph)。
//!
//! 从对话/文档中抽取实体、关系与 episode,合并去重,支持语义检索
//! (实体/关系/episode)与邻居遍历。契约零实现;真实后端由插件提供。

use serde_json::Value;

use crate::seam::Seam;

/// 实体(图节点)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Entity {
    pub uuid: String,
    pub name: String,
    pub content: String,
    /// 关联的关系 uuid。
    #[serde(default)]
    pub relations: Vec<String>,
    /// 出现的 episode uuid。
    #[serde(default)]
    pub episodes: Vec<String>,
    /// 附加属性。
    #[serde(default)]
    pub attributes: Value,
}

/// 关系(图边)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Relation {
    pub uuid: String,
    pub name: String,
    /// 左端实体 uuid。
    pub lhs: String,
    /// 右端实体 uuid。
    pub rhs: String,
    pub content: String,
}

/// 记忆片段(无名字节点,记录实体共现)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Episode {
    pub uuid: String,
    pub content: String,
    /// 本片段提到的实体 uuid。
    #[serde(default)]
    pub entities: Vec<String>,
}

/// 检索命中。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GraphHit {
    /// "entity" | "relation" | "episode"。
    pub kind: String,
    pub uuid: String,
    pub name: String,
    pub content: String,
    /// 关键词匹配分(命中数)。
    pub score: u64,
}

/// 添加记忆的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AddMemoryResult {
    /// 新增实体名。
    pub entities: Vec<String>,
    /// 新增关系描述。
    pub relations: Vec<String>,
    /// 新增 episode uuid。
    pub episode: String,
}

/// graph memory 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMemoryError(pub String);

impl core::fmt::Display for GraphMemoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GraphMemoryError {}

/// graph memory Seam(Service Definition):知识图谱记忆。
///
/// 实现方(插件)提供真实抽取/存储/检索;消费方(agent 工具)只依赖本 trait。
pub trait GraphMemory: Seam {
    /// 添加一段记忆:抽取实体/关系,建 episode,与已有图合并去重。
    fn add_memory(&self, content: &str) -> Result<AddMemoryResult, GraphMemoryError>;

    /// 关键词检索(实体/关系/episode 统一命中,按得分降序)。
    fn search(&self, query: &str) -> Result<Vec<GraphHit>, GraphMemoryError>;

    /// 邻居遍历:与指定实体相连的全部关系(含方向)。
    fn neighbors(&self, entity_uuid: &str) -> Result<Vec<Relation>, GraphMemoryError>;

    /// 按名取实体(精确)。
    fn entity_by_name(&self, name: &str) -> Option<Entity>;

    /// 全部实体(按名字排序)。
    fn entities(&self) -> Vec<Entity>;

    /// 全部关系。
    fn relations(&self) -> Vec<Relation>;

    /// 全部 episode(按 uuid 排序)。
    fn episodes(&self) -> Vec<Episode>;
}
