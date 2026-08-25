//! 确定性训练工具门面(对齐 dev_tools/tune 的确定性部分)。
//!
//! 委托 ah-contracts tune_kit 纯函数:CaseLoader(shuffle/split/case_id)、
//! 参数校验、json/list 块解析、examples 格式化、优化 prompt 标签提取、
//! 占位符查找与缺失计算、评估结果 → 分数。

use std::sync::Arc;

use ah_contracts::keys::TUNE_KIT;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tune_kit::{
    Case, CaseLoader, EvaluatedCase, TuneKit, TuneKitError, TuneUtils,
    extract_optimized_prompt_from_response, find_missing_placeholders,
    find_placeholders_from_prompt,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 纯函数门面实现:委托契约层。
pub struct TuneKitImpl;

impl Seam for TuneKitImpl {}

impl TuneKit for TuneKitImpl {
    fn case_loader(&self, cases: Vec<Case>) -> CaseLoader {
        CaseLoader::new(cases)
    }

    fn validate_digital_parameter(
        &self,
        param: f64,
        param_name: &str,
        lower: f64,
        upper: f64,
    ) -> Result<(), TuneKitError> {
        TuneUtils::validate_digital_parameter(param, param_name, lower, upper)
    }

    fn parse_json_from_llm_response(&self, json_like: &str) -> Option<Value> {
        TuneUtils::parse_json_from_llm_response(json_like)
    }

    fn parse_list_from_llm_response(&self, list_like: &str) -> Option<Vec<Value>> {
        TuneUtils::parse_list_from_llm_response(list_like)
    }

    fn convert_cases_to_examples(&self, cases: &[EvaluatedCase]) -> String {
        TuneUtils::convert_cases_to_examples(cases)
    }

    fn extract_optimized_prompt(&self, response: &str, tag: &str) -> Option<String> {
        extract_optimized_prompt_from_response(response, tag)
    }

    fn find_placeholders(&self, prompt: &str) -> Vec<String> {
        find_placeholders_from_prompt(prompt)
    }

    fn missing_placeholders(&self, original: &[String], optimized: &[String]) -> Vec<String> {
        find_missing_placeholders(original, optimized)
    }

    fn score_from_result(&self, result: &Value) -> f64 {
        ah_contracts::tune_kit::evaluate_result_to_score(result)
    }
}

/// tune-kit 插件:注册 `tune-kit` seam(确定性训练工具门面)。
pub struct TuneKitPlugin;

impl Plugin for TuneKitPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tune-kit"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![TUNE_KIT]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let kit: Arc<dyn TuneKit> = Arc::new(TuneKitImpl);
        Ok(vec![ctx.register(TUNE_KIT, kit)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::TUNE_KIT;
    use ah_contracts::tune_kit::tune_constant;
    use ah_hub::plugin::DynPlugin;
    use serde_json::{Map, json};

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(TuneKitPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered_and_loader_works() {
        let (ctx, effects) = build_ctx();
        let kit = ctx.service::<dyn TuneKit>(&TUNE_KIT).expect("tune-kit");
        let cases = vec![
            Case::new(
                Map::from_iter([("q".into(), json!("a"))]),
                Map::from_iter([("l".into(), json!("1"))]),
            ),
            Case::new(
                Map::from_iter([("q".into(), json!("b"))]),
                Map::from_iter([("l".into(), json!("2"))]),
            ),
        ];
        let mut loader = kit.case_loader(cases);
        assert_eq!(loader.size(), 2);
        assert_eq!(loader.get_cases()[0].case_id, "case_0");
        loader.shuffle(42);
        assert_eq!(loader.get_cases()[0].case_id, "case_0");
        drop(effects);
    }

    #[test]
    fn extraction_and_validation_via_seam() {
        let (ctx, effects) = build_ctx();
        let kit = ctx.service::<dyn TuneKit>(&TUNE_KIT).expect("tune-kit");
        // json 块解析。
        let parsed = kit
            .parse_json_from_llm_response("```json\n{\"result\": true}\n```")
            .expect("json");
        assert_eq!(parsed["result"], true);
        // 优化 prompt 提取。
        let extracted = kit
            .extract_optimized_prompt(
                "<PROMPT_OPTIMIZED>\nnew prompt\n</PROMPT_OPTIMIZED>",
                "PROMPT_OPTIMIZED",
            )
            .expect("tag");
        assert_eq!(extracted, "\nnew prompt\n", "Python regex 捕获不 strip");
        // 占位符。
        let found = kit.find_placeholders("Task {{a.b}} and {{c}}");
        assert_eq!(found, vec!["a.b", "c"]);
        let missing = kit.missing_placeholders(&found, &["a.b".to_string()]);
        assert_eq!(missing, vec!["c"]);
        // 参数校验 + 常量。
        assert!(
            kit.validate_digital_parameter(5.0, "num_parallel", 1.0, 20.0)
                .is_ok()
        );
        assert_eq!(tune_constant::DEFAULT_ITERATION_NUM, 3);
        drop(effects);
    }
}
