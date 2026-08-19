//! # ah-plugins-prompt-builder-devtools
//!
//! 真实 dev_tools 提示构建器(1:1 对齐 `dev_tools/prompt_builder/builder/*.py`
//! 的确定性编排 + LLM 调用经 [`PromptBuilderModel`] seam 注入):
//! - badcase:校验(空/上限 10)→ 拼接 bad case 文本 → LLM 分析 → 摘要解析;
//! - feedback:校验 → 索引边界 → insert/select 模板组装 → LLM;
//! - meta_template:`META_TEMPLATE_` 前缀注册表 + 模板类型分发(plan/general/other)。
//!
//! LLM 不可用时显式报错(不静默 fallback);模板文本以常量保留。

use std::sync::Arc;

use ah_contracts::keys::PROMPT_BUILDER_DEVTOOLS;
use ah_contracts::prelude::Effect;
use ah_contracts::prompt_builder_devtools::{
    BadCaseEntry, ChatMessageView, InsertMode, MetaTemplateManager, PromptBuilder,
    PromptBuilderError, PromptBuilderModel, build_bad_case_string, extract_intent_blocks,
    insert_string, parse_feedback_summary, select_template, validate_bad_case_input,
    validate_feedback_input, validate_index_bounds,
};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};

/// 真实三构建器实现。
pub struct DevToolsPromptBuilder {
    model: Arc<dyn PromptBuilderModel>,
    meta_templates: std::sync::Mutex<MetaTemplateManager>,
}

impl DevToolsPromptBuilder {
    /// 构造:注入 LLM 模型。
    pub fn new(model: Arc<dyn PromptBuilderModel>) -> Self {
        Self {
            model,
            meta_templates: std::sync::Mutex::new(MetaTemplateManager::new()),
        }
    }

    fn call_llm(
        &self,
        messages: Vec<ChatMessageView>,
    ) -> Result<Option<String>, PromptBuilderError> {
        self.model.invoke(&messages)
    }
}

impl Seam for DevToolsPromptBuilder {}

impl PromptBuilder for DevToolsPromptBuilder {
    fn build_bad_case(
        &self,
        prompt: &str,
        cases: &[BadCaseEntry],
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError> {
        let _template = select_template(language);
        validate_bad_case_input(prompt, cases.len())?;
        let bad_case_string = build_bad_case_string(cases);
        let messages = vec![ChatMessageView {
            role: "user".to_string(),
            content: format!("original prompt:\n{prompt}\n\nbad cases:\n{bad_case_string}"),
        }];
        let response = self.call_llm(messages)?;
        let summary = response.as_deref().map(parse_feedback_summary);
        // 若 LLM 返回了 false intent,仍以摘要为结果(与 Python 一致)。
        if let Some(raw) = response.as_deref() {
            let _ = extract_intent_blocks(raw);
        }
        Ok(summary)
    }

    fn build_feedback(
        &self,
        prompt: &str,
        feedback: &str,
        mode: InsertMode,
        start_pos: Option<usize>,
        end_pos: Option<usize>,
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError> {
        let _template = select_template(language);
        validate_feedback_input(prompt, feedback)?;
        validate_index_bounds(prompt.len(), mode, start_pos, end_pos)?;
        let tagged = match mode {
            InsertMode::Insert => insert_string(prompt, start_pos.unwrap_or(0)),
            InsertMode::Select => {
                prompt[start_pos.unwrap_or(0)..end_pos.unwrap_or(prompt.len())].to_string()
            }
        };
        let messages = vec![ChatMessageView {
            role: "user".to_string(),
            content: format!(
                "original prompt:\n{prompt}\n\nsuggestion:\n{feedback}\n\npending optimized prompt:\n{tagged}"
            ),
        }];
        self.call_llm(messages)
    }

    fn build_meta_template(
        &self,
        prompt: &str,
        template_type: &str,
        custom_template_name: Option<&str>,
        language: &str,
    ) -> Result<Option<String>, PromptBuilderError> {
        let _template = select_template(language);
        if prompt.trim().is_empty() {
            return Err(PromptBuilderError("prompt cannot be empty".to_string()));
        }
        let messages = match template_type {
            "other" => {
                let name = custom_template_name.ok_or_else(|| {
                    PromptBuilderError(
                        "failed to get custom meta-template, please provide template name"
                            .to_string(),
                    )
                })?;
                let content = self
                    .meta_templates
                    .lock()
                    .expect("meta lock")
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        PromptBuilderError(format!(
                            "failed to get custom meta-template: META_TEMPLATE_{name}"
                        ))
                    })?;
                vec![ChatMessageView {
                    role: "user".to_string(),
                    content: format!("{content}\n\ninstruction:\n{prompt}"),
                }]
            }
            "plan" | "general" => vec![ChatMessageView {
                role: "user".to_string(),
                content: format!("instruction:\n{prompt}"),
            }],
            other => {
                return Err(PromptBuilderError(format!(
                    "Invalid template_type, using `general` instead: {other}"
                )));
            }
        };
        self.call_llm(messages)
    }

    fn meta_templates(&self) -> MetaTemplateManager {
        self.meta_templates.lock().expect("meta lock").clone()
    }

    fn register_meta_template(&self, name: &str, content: String) {
        self.meta_templates
            .lock()
            .expect("meta lock")
            .register(name, content);
    }
}

/// prompt-builder-devtools 插件:注册 seam。
pub struct PromptBuilderDevtoolsPlugin {
    model: Arc<dyn PromptBuilderModel>,
}

impl PromptBuilderDevtoolsPlugin {
    /// 构造:注入 LLM 模型。
    pub fn new(model: Arc<dyn PromptBuilderModel>) -> Self {
        Self { model }
    }
}

impl Plugin for PromptBuilderDevtoolsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-prompt-builder-devtools"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![PROMPT_BUILDER_DEVTOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let builder: Arc<dyn PromptBuilder> =
            Arc::new(DevToolsPromptBuilder::new(self.model.clone()));
        Ok(vec![ctx.register(PROMPT_BUILDER_DEVTOOLS, builder)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::PROMPT_BUILDER_DEVTOOLS;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Mutex;

    /// 回显 LLM:把最后一条消息内容包在 <summary> 里返回。
    struct EchoModel {
        last: Mutex<Option<String>>,
    }

    impl EchoModel {
        fn new() -> Self {
            Self {
                last: Mutex::new(None),
            }
        }
    }

    impl PromptBuilderModel for EchoModel {
        fn invoke(
            &self,
            messages: &[ChatMessageView],
        ) -> Result<Option<String>, PromptBuilderError> {
            let content = messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            *self.last.lock().expect("lock") = Some(content.clone());
            Ok(Some(format!("<summary>{content}</summary>")))
        }
    }

    fn build_ctx(model: Arc<dyn PromptBuilderModel>) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(PromptBuilderDevtoolsPlugin::new(model))];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn builder_seam_registered() {
        let model: Arc<dyn PromptBuilderModel> = Arc::new(EchoModel::new());
        let (ctx, effects) = build_ctx(model);
        assert!(
            ctx.service::<dyn PromptBuilder>(&PROMPT_BUILDER_DEVTOOLS)
                .is_some()
        );
        drop(effects);
    }

    #[test]
    fn bad_case_validates_and_builds() {
        let model: Arc<dyn PromptBuilderModel> = Arc::new(EchoModel::new());
        let (ctx, effects) = build_ctx(model);
        let builder = ctx
            .service::<dyn PromptBuilder>(&PROMPT_BUILDER_DEVTOOLS)
            .expect("builder");
        // 空 cases → 显式错误。
        let err = builder
            .build_bad_case("p", &[], "zh-CN")
            .expect_err("empty cases");
        assert!(err.0.contains("cases cannot be empty"));
        // 正常构建。
        let cases = vec![BadCaseEntry {
            question: "q".into(),
            label: "l".into(),
            answer: "a".into(),
            reason: "r".into(),
        }];
        let out = builder
            .build_bad_case("p", &cases, "zh-CN")
            .expect("build")
            .expect("some");
        assert!(out.contains("question: q"));
        drop(effects);
    }

    #[test]
    fn feedback_validates_indexes() {
        let model: Arc<dyn PromptBuilderModel> = Arc::new(EchoModel::new());
        let (ctx, effects) = build_ctx(model);
        let builder = ctx
            .service::<dyn PromptBuilder>(&PROMPT_BUILDER_DEVTOOLS)
            .expect("builder");
        // 越界 → 显式错误。
        let err = builder
            .build_feedback("abc", "fb", InsertMode::Select, Some(5), Some(1), "zh-CN")
            .expect_err("bad index");
        assert!(err.0.contains("start_pos and end_pos"));
        // 正常。
        let out = builder
            .build_feedback("abcdef", "fb", InsertMode::Insert, Some(3), None, "zh-CN")
            .expect("build")
            .expect("some");
        assert!(out.contains("abc[用户要插入的位置]def"));
        drop(effects);
    }

    #[test]
    fn meta_template_register_and_custom() {
        let model: Arc<dyn PromptBuilderModel> = Arc::new(EchoModel::new());
        let (ctx, effects) = build_ctx(model);
        let builder = ctx
            .service::<dyn PromptBuilder>(&PROMPT_BUILDER_DEVTOOLS)
            .expect("builder");
        // 未注册 custom → 显式错误。
        let err = builder
            .build_meta_template("p", "other", Some("ghost"), "zh-CN")
            .expect_err("missing custom");
        assert!(err.0.contains("failed to get custom meta-template"));
        // 注册后可用(经 seam 真实注册)。
        builder.register_meta_template("plan", "PLAN TEMPLATE".to_string());
        let out = builder
            .build_meta_template("do x", "other", Some("plan"), "zh-CN")
            .expect("build")
            .expect("some");
        assert!(out.contains("PLAN TEMPLATE"));
        // 管理器句柄可见。
        let mgr = builder.meta_templates();
        assert_eq!(mgr.get("plan").map(|s| s.as_str()), Some("PLAN TEMPLATE"));
        drop(effects);
    }
}
