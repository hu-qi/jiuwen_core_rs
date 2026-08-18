//! # ah-plugins-tools-metadata
//!
//! Real bilingual tool metadata providers (aligned with
//! harness/prompts/tools/*.py): descriptions + input param schemas for
//! harness built-in tools. Metadata is validated against the
//! ToolMetadataProvider contract (validate_provider) at build time.

pub mod basic;
pub mod memory_tools;
pub mod special;

use std::sync::Arc;

use ah_contracts::keys::TOOLS_METADATA;
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::ToolMetadata;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 汇总全部工具元数据(供注册/校验)。
pub fn all_metadata() -> Vec<ToolMetadata> {
    let mut all = Vec::new();
    all.extend(basic::metadata());
    all.extend(memory_tools::metadata());
    all.extend(special::metadata());
    all
}

/// 元数据集(注册用;对齐 metadata registry 语义)。
pub struct MetadataSet(pub Vec<ToolMetadata>);

impl ah_contracts::seam::Seam for MetadataSet {}

/// tools-metadata 插件:注册元数据服务。
pub struct ToolsMetadataPlugin;

impl Plugin for ToolsMetadataPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tools-metadata"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TOOLS_METADATA]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let metas = Arc::new(MetadataSet(all_metadata()));
        Ok(vec![ctx.register(TOOLS_METADATA, metas)])
    }
}
