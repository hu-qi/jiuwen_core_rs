//! prompt seam:模板渲染与版本化 prompt 注册表。
//!
//! 对应 openjiuwen/core 的 prompt_builder:{{var}} 占位渲染、缺失变量显式报错
//! (不静默),同名注册递增版本。

use std::collections::HashMap;

use crate::seam::Seam;

/// 一个 prompt 模板。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    /// 同名单次注册递增;最新版本为当前生效版本。
    pub version: u32,
    /// 模板文本,占位符格式 {{var}}。
    pub template: String,
    pub description: String,
}

/// 渲染结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RenderedPrompt {
    pub name: String,
    pub version: u32,
    pub content: String,
    /// 模板中出现的变量名。
    pub variables: Vec<String>,
}

/// prompt 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptError(pub String);

impl core::fmt::Display for PromptError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PromptError {}

/// prompt Seam(Service Definition):注册/查询/渲染。
///
/// 渲染规则:{{var}} 占位符以给定变量替换;缺失变量返回显式错误并列出缺失项。
pub trait PromptRegistry: Seam {
    /// 注册模板;同名注册递增版本并覆盖生效。
    fn register(&self, template: PromptTemplate) -> Result<PromptTemplate, PromptError>;

    /// 取最新版本模板。
    fn get(&self, name: &str) -> Option<PromptTemplate>;

    /// 全部模板(按名称排序,每名取最新版本)。
    fn list(&self) -> Vec<PromptTemplate>;

    /// 渲染最新版本;缺失变量显式报错。
    fn render(
        &self,
        name: &str,
        vars: &HashMap<String, String>,
    ) -> Result<RenderedPrompt, PromptError>;
}
