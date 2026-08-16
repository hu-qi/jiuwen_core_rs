//! sharing seam:经验分享(对齐 openjiuwen/agent_evolving/sharing)。
//!
//! - `SharedExperience`:一条可分享经验(包装 evolving `Experience`,附加 keywords/summary/SharingMeta);
//! - `SharedSkillBundle`:后端存储的最小单元(一个 skill 的多条经验 + 聚合关键词/摘要);
//! - `SharingBackend`:存储后端契约(upload/download 束、skill 包、关键词检索);
//! - `ExperienceSharer`:skill 级外观(stage 去重队列 → flush 打包上传 + 重试/退避;download 镜像本地缓存)。
//!
//! 契约零实现:存储与上传逻辑由插件提供(如 ah-plugins-sharing 的本地文件后端 + 外观实现)。

use std::sync::Arc;

use async_trait::async_trait;

use crate::evolving::Experience;
use crate::seam::Seam;

/// 每条经验分享侧元数据。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SharingMeta {
    pub skill_name: String,
    pub skill_version: String,
    pub upload_trigger: String,
    pub upload_at: String,
    pub feedback_excerpt: Option<String>,
    pub source_user_id: Option<String>,
    pub confidence: f64,
    pub origin_bundle_id: Option<String>,
}

impl SharingMeta {
    /// 默认:user_approval 触发、confidence 0.7。
    pub fn new(skill_name: impl Into<String>) -> Self {
        Self {
            skill_name: skill_name.into(),
            skill_version: String::new(),
            upload_trigger: "user_approval".to_string(),
            upload_at: String::new(),
            feedback_excerpt: None,
            source_user_id: None,
            confidence: 0.7,
            origin_bundle_id: None,
        }
    }
}

/// 可分享经验:包装一条 evolving `Experience` + 分享侧字段。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SharedExperience {
    pub record: Experience,
    pub keywords: Vec<String>,
    pub summary: String,
    pub sharing_meta: Option<SharingMeta>,
}

impl SharedExperience {
    pub fn new(record: Experience) -> Self {
        Self {
            record,
            keywords: Vec::new(),
            summary: String::new(),
            sharing_meta: None,
        }
    }
}

/// hub 上单个不可变 skill 包的元数据。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillPackageMeta {
    pub skill_id: String,
    pub skill_name: String,
    pub description: String,
    pub uploaded_at: String,
}

impl SkillPackageMeta {
    pub fn new(skill_id: impl Into<String>) -> Self {
        Self {
            skill_id: skill_id.into(),
            skill_name: String::new(),
            description: String::new(),
            uploaded_at: String::new(),
        }
    }
}

/// hub 关键词检索的一行结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillSearchResult {
    pub skill_id: String,
    pub skill_name: String,
    pub description: String,
    pub experience_count: u64,
    pub keywords: Vec<String>,
    pub score: f64,
}

/// 束是后端存储的最小单元:一个 skill 的多条经验。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SharedSkillBundle {
    pub bundle_id: String,
    pub skill_id: String,
    pub skill_name: String,
    pub skill_version: String,
    pub keywords_aggregate: Vec<String>,
    pub summary_aggregate: String,
    pub experiences: Vec<SharedExperience>,
    pub created_at: String,
}

impl SharedSkillBundle {
    /// 由 skill 名与经验构造束:关键词去重保序聚合;摘要以 "; " 连接。
    pub fn make(
        skill_name: impl Into<String>,
        experiences: Vec<SharedExperience>,
        skill_version: impl Into<String>,
        summary_aggregate: impl Into<String>,
    ) -> Self {
        let mut seen: Vec<String> = Vec::new();
        for exp in &experiences {
            for kw in &exp.keywords {
                if !kw.is_empty() && !seen.contains(kw) {
                    seen.push(kw.clone());
                }
            }
        }
        let summary = summary_aggregate.into();
        let summary = if summary.is_empty() {
            experiences
                .iter()
                .filter(|e| !e.summary.is_empty())
                .map(|e| e.summary.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        } else {
            summary
        };
        Self {
            bundle_id: String::new(), // 由后端/实现生成
            skill_id: String::new(),
            skill_name: skill_name.into(),
            skill_version: skill_version.into(),
            keywords_aggregate: seen,
            summary_aggregate: summary,
            experiences,
            created_at: String::new(),
        }
    }
}

/// 查询侧关键词集(用于检索相关束)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueryKeywords {
    pub keywords: Vec<String>,
    pub intent: String,
    pub raw_excerpt: String,
}

impl QueryKeywords {
    pub fn new(keywords: Vec<String>) -> Self {
        Self {
            keywords,
            intent: String::new(),
            raw_excerpt: String::new(),
        }
    }
}

/// 束上传结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UploadResult {
    pub ok: bool,
    pub bundle_id: String,
    pub reason: String,
    pub retryable: bool,
}

impl UploadResult {
    pub fn ok(bundle_id: impl Into<String>) -> Self {
        Self {
            ok: true,
            bundle_id: bundle_id.into(),
            reason: String::new(),
            retryable: false,
        }
    }

    pub fn rejected(reason: impl Into<String>, retryable: bool) -> Self {
        Self {
            ok: false,
            bundle_id: String::new(),
            reason: reason.into(),
            retryable,
        }
    }
}

/// sharing 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharingError(pub String);

impl core::fmt::Display for SharingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SharingError {}

/// 存储后端契约:束的持久化 / skill 包 / 关键词检索。
#[async_trait]
pub trait SharingBackend: Seam {
    /// 持久化束(接受时返回 ok)。
    async fn upload_bundle(&self, bundle: SharedSkillBundle) -> Result<UploadResult, SharingError>;

    /// 按查询相关度返回最多 top_k 个束。
    async fn download_bundles(
        &self,
        skill_id: &str,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SharedSkillBundle>, SharingError>;

    /// hub 是否已存有该 skill 的初始包。
    async fn has_skill_package(&self, skill_id: &str) -> Result<bool, SharingError>;

    /// 持久化初始 skill 包;已存在时视为 no-op(远端只保留第一版)。
    async fn upload_skill_package(
        &self,
        skill_id: &str,
        package_bytes: &[u8],
        meta: &SkillPackageMeta,
    ) -> Result<(), SharingError>;

    /// 返回 skill 包字节;缺失返回 None。
    async fn download_skill_package(&self, skill_id: &str)
    -> Result<Option<Vec<u8>>, SharingError>;

    /// 返回 hub 上该 skill 的元数据。
    async fn get_skill_package_meta(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillPackageMeta>, SharingError>;

    /// 按关键词相关度搜索 skill。
    async fn search_skills(
        &self,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SkillSearchResult>, SharingError>;
}

/// skill 分享上下文提供者:返回 (skill_id, package_bytes, skill_name, description)。
pub trait SkillSharingContextProvider: Send + Sync {
    fn provide(&self, skill_name: &str) -> Result<(String, Vec<u8>, String, String), String>;
}

/// skill 级分享外观:stage 队列 → flush 打包上传(重试/退避)+ 本地镜像缓存。
#[async_trait]
pub trait ExperienceSharer: Seam {
    /// 解析本地 skill 的 skill_id(provider 可用时;否则空串)。
    fn resolve_skill_id(&self, skill_name: &str) -> String;

    /// 该 skill 是否还有待上传的经验。
    fn has_pending(&self, skill_name: &str) -> bool;

    /// 排队待上传(按 (skill, record.id) 去重)。
    fn stage_for_upload(&self, skill_name: &str, exp: SharedExperience);

    /// 丢弃该 skill 的待上传队列(负反馈),返回丢弃条数。
    fn discard_pending_uploads(&self, skill_name: &str) -> usize;

    /// 打包并上传该 skill 的全部待上传经验(含 skill 包同步与重试)。
    async fn flush_pending_uploads(&self, skill_name: &str) -> Result<UploadResult, SharingError>;

    /// 下载相关束并镜像到本地缓存。
    async fn download_relevant(
        &self,
        skill_id: &str,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SharedSkillBundle>, SharingError>;

    /// hub 关键词搜索。
    async fn search_skills(
        &self,
        query: &QueryKeywords,
        top_k: usize,
    ) -> Result<Vec<SkillSearchResult>, SharingError>;

    /// 下载不可变 skill 包。
    async fn download_skill_package(&self, skill_id: &str)
    -> Result<Option<Vec<u8>>, SharingError>;

    /// 获取 skill 包元数据。
    async fn get_skill_package_meta(
        &self,
        skill_id: &str,
    ) -> Result<Option<SkillPackageMeta>, SharingError>;

    /// 本地下载缓存中的束列表。
    fn list_cached_bundles(&self, skill_id: &str) -> Vec<SharedSkillBundle>;
}

/// 共享的后端引用(供实现内部使用)。
pub type SharedBackend = Arc<dyn SharingBackend>;

/// 共享的 skill 上下文提供者引用。
pub type SharedSkillProvider = Arc<dyn SkillSharingContextProvider>;
