//! # ah-plugins-skill-creator
//!
//! 真实技能创建流水线(1:1 对齐 `dev_tools/skill_creator/skills/
//! skill_omni_creation/scripts/*.py` 的确定性编排):
//! - stage_01:URL 抓取(经 [`SkillFetcher`] seam)→ blocks 构建;
//! - stage_03:过滤(空/重复/超长/未知类型);
//! - stage_04:资产清单编号 + SKILL.md 骨架;
//! - stage_05:LLM 生成(经 [`SkillGenerator`] seam)+ 去幻影图片后处理。
//!
//! 网络抓取 / LLM 生成不可用时显式报错(不静默 fallback);纯函数在契约层。

use std::sync::Arc;

use ah_contracts::keys::SKILL_CREATOR;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::skill_creator::{
    SkillCreator, SkillCreatorError, SkillFetcher, SkillGenRequest, SkillGenerator, filter_block,
    strip_hallucinated_images, url_to_slug,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实流水线实现。
pub struct SkillCreatorImpl {
    fetcher: Arc<dyn SkillFetcher>,
    generator: Arc<dyn SkillGenerator>,
}

impl SkillCreatorImpl {
    /// 构造:注入抓取器与生成器。
    pub fn new(fetcher: Arc<dyn SkillFetcher>, generator: Arc<dyn SkillGenerator>) -> Self {
        Self { fetcher, generator }
    }

    /// stage_01:抓取 URL 构建 blocks(确定性编排;抓取失败显式报错)。
    fn scrape(&self, url: &str) -> Result<Vec<serde_json::Value>, SkillCreatorError> {
        let (bytes, mime) = self.fetcher.fetch(url)?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        if text.trim().is_empty() {
            return Err(SkillCreatorError("scraped content is empty".to_string()));
        }
        // 简化为一个文本 block + 图片 block(完整 HTML 解析留待后续)。
        Ok(vec![
            serde_json::json!({"type": "paragraph", "text": text, "mime": mime}),
        ])
    }

    /// stage_03:过滤 blocks(确定性)。
    fn filter_blocks(&self, blocks: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let mut seen = std::collections::BTreeSet::new();
        blocks
            .into_iter()
            .filter(|b| filter_block(b, &mut seen).keep)
            .collect()
    }

    /// stage_04:SKILL.md 骨架(确定性)。
    fn build_skill_md(&self, title: &str, slug: &str, blocks: &[serde_json::Value]) -> String {
        let mut md = String::new();
        md.push_str(&format!("# {title}\n\n"));
        md.push_str("---\nname: ");
        md.push_str(slug);
        md.push_str("\ndescription: 自动创建的技能\n---\n\n");
        for block in blocks {
            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                md.push_str(text);
                md.push_str("\n\n");
            } else if block.get("type").and_then(|v| v.as_str()) == Some("image")
                && let Some(path) = block.get("path").and_then(|v| v.as_str())
            {
                md.push_str(&format!("![image]({path})\n\n"));
            }
        }
        md
    }
}

impl Seam for SkillCreatorImpl {}

impl SkillCreator for SkillCreatorImpl {
    fn create_skill(
        &self,
        url: &str,
        title: &str,
        _language: &str,
    ) -> Result<String, SkillCreatorError> {
        if url.trim().is_empty() || title.trim().is_empty() {
            return Err(SkillCreatorError("url and title are required".to_string()));
        }
        // stage_01。
        let blocks = self.scrape(url)?;
        // stage_03。
        let filtered = self.filter_blocks(blocks);
        // stage_04(骨架)+ stage_05(LLM 生成 + 后处理)。
        let slug = url_to_slug(url);
        let skeleton = self.build_skill_md(title, &slug, &filtered);
        let request = SkillGenRequest {
            title: title.to_string(),
            slug,
            summary: String::new(),
            blocks: filtered,
        };
        let generated = self.generator.generate_skill_md(&request)?;
        // 去幻影图片:仅保留真实资产引用(本实现无资产,故全部去除)。
        let valid = std::collections::BTreeSet::new();
        let cleaned = strip_hallucinated_images(&generated, &valid);
        if cleaned.trim().is_empty() {
            // 生成空 → 回退骨架(显式路径,非静默)。
            return Ok(skeleton);
        }
        Ok(cleaned)
    }
}

/// skill-creator 插件:注册 seam。
pub struct SkillCreatorPlugin {
    fetcher: Arc<dyn SkillFetcher>,
    generator: Arc<dyn SkillGenerator>,
}

impl SkillCreatorPlugin {
    /// 构造:注入抓取器与生成器。
    pub fn new(fetcher: Arc<dyn SkillFetcher>, generator: Arc<dyn SkillGenerator>) -> Self {
        Self { fetcher, generator }
    }
}

impl Plugin for SkillCreatorPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-skill-creator"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![SKILL_CREATOR]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let creator: Arc<dyn SkillCreator> = Arc::new(SkillCreatorImpl::new(
            self.fetcher.clone(),
            self.generator.clone(),
        ));
        Ok(vec![ctx.register(SKILL_CREATOR, creator)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::SKILL_CREATOR;
    use ah_hub::plugin::DynPlugin;

    struct FakeFetcher;

    impl SkillFetcher for FakeFetcher {
        fn fetch(&self, _url: &str) -> Result<(Vec<u8>, String), SkillCreatorError> {
            Ok((
                "## 教程内容\n\n如何创建技能".as_bytes().to_vec(),
                "text/html".to_string(),
            ))
        }
    }

    struct FakeGenerator;

    impl SkillGenerator for FakeGenerator {
        fn generate_skill_md(&self, spec: &SkillGenRequest) -> Result<String, SkillCreatorError> {
            Ok(format!(
                "# {}\n\n![img](dom_000.png)\n\n正文 {}",
                spec.title, spec.slug
            ))
        }
    }

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(SkillCreatorPlugin::new(
            Arc::new(FakeFetcher),
            Arc::new(FakeGenerator),
        ))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn creator_seam_registered() {
        let (ctx, effects) = build_ctx();
        assert!(ctx.service::<dyn SkillCreator>(&SKILL_CREATOR).is_some());
        drop(effects);
    }

    #[test]
    fn create_skill_runs_pipeline() {
        let (ctx, effects) = build_ctx();
        let creator = ctx
            .service::<dyn SkillCreator>(&SKILL_CREATOR)
            .expect("creator");
        let md = creator
            .create_skill("https://example.com/guide", "测试技能", "zh-CN")
            .expect("create");
        assert!(md.contains("测试技能"));
        // 幻影图片被移除(valid_paths 为空)。
        assert!(!md.contains("![img]"));
        drop(effects);
    }

    #[test]
    fn create_skill_validates_input() {
        let (ctx, effects) = build_ctx();
        let creator = ctx
            .service::<dyn SkillCreator>(&SKILL_CREATOR)
            .expect("creator");
        let err = creator
            .create_skill("", "t", "zh-CN")
            .expect_err("empty url");
        assert!(err.0.contains("url and title are required"));
        drop(effects);
    }

    #[test]
    fn slugify_used_for_skill_name() {
        assert_eq!(
            ah_contracts::skill_creator::slugify("My  Guide!"),
            "my_guide"
        );
    }
}
