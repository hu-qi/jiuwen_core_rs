//! # ah-plugins-prompt-builder
//!
//! Real section-based system prompt builder (aligned with
//! core/single_agent/prompts/builder.py + harness/prompts/{sanitize,report}.py):
//! - builder: multilingual PromptSection registry, priority-ordered rendering,
//!   DeepAgent modes (full/minimal/none) with minimal-section filter;
//! - sanitize: prompt-injection defense (strip <>{}[]`\$ and ellipsis/newlines);
//! - report: PromptReport diagnostics (chars / est. tokens / section stats).

pub mod builder;
pub mod report;
pub mod sanitize;
pub mod sections;

use std::sync::Arc;

use ah_contracts::keys::PROMPT_BUILDER;
use ah_contracts::prelude::Effect;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// prompt-builder 插件:注册构建器服务。
pub struct PromptBuilderPlugin;

impl Plugin for PromptBuilderPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-prompt-builder"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        vec![PROMPT_BUILDER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let builder = Arc::new(builder::SystemPromptBuilder::new("cn", Default::default()));
        Ok(vec![ctx.register(PROMPT_BUILDER, builder)])
    }
}
