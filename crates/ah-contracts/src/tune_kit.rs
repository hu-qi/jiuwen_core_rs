//! tune-kit seam:dev_tools/tune 的确定性数据模型与纯函数
//! (对齐 `openjiuwen/dev_tools/tune/` 的确定性部分)。
//!
//! - `TuneConstant` 常量 / `Case` / `EvaluatedCase`(对齐 base.py);
//! - `CaseLoader`(shuffle/split/assign_case_id,对齐 dataset/case_loader.py);
//! - `TuneUtils` 纯函数(参数校验 / json·list 提取 / examples 格式化,
//!   对齐 utils.py);
//! - `TextualParameter` / `TraceNode` / `OptimizeHistory`(对齐
//!   optimizer/base.py);
//! - `Progress`(run_epoch/run_batch,对齐 trainer/base.py);
//! - 确定性提取与校验:优化 prompt 标签提取 / 占位符查找与缺失计算 /
//!   bad-case 文本格式化 / 评估结果 → 分数(对齐 instruction_optimizer.py /
//!   evaluator.py 的确定性部分)。
//!
//! 纯函数、无 IO、无 LLM;契约零实现。LLM 调用(梯度生成/优化/选择)经
//! seam 注入。

use crate::seam::Seam;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// 训练常量(对齐 TuneConstant)。
pub mod tune_constant {
    pub const DEFAULT_EXAMPLE_NUM: usize = 1;
    pub const DEFAULT_ITERATION_NUM: u32 = 3;
    pub const DEFAULT_MAX_SAMPLED_EXAMPLE_NUM: usize = 10;
    pub const DEFAULT_PARALLEL_NUM: usize = 1;
    pub const DEFAULT_MAX_NUM_SAMPLE_ERROR_CASES: usize = 10;
    pub const DEFAULT_EARLY_STOP_SCORE: f64 = 1.0;
    pub const MIN_ITERATION_NUM: u32 = 1;
    pub const MAX_ITERATION_NUM: u32 = 20;
    pub const MIN_PARALLEL_NUM: usize = 1;
    pub const MAX_PARALLEL_NUM: usize = 20;
    pub const MIN_EXAMPLE_NUM: usize = 0;
    pub const MAX_EXAMPLE_NUM: usize = 20;
}

/// 一个训练用例(对齐 Case)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Case {
    pub inputs: Map<String, Value>,
    pub label: Map<String, Value>,
    #[serde(default)]
    pub tools: Option<Vec<Value>>,
    #[serde(default)]
    pub case_id: String,
}

impl Case {
    pub fn new(inputs: Map<String, Value>, label: Map<String, Value>) -> Self {
        Self {
            inputs,
            label,
            tools: None,
            case_id: String::new(),
        }
    }
}

/// 已评估用例(对齐 EvaluatedCase)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluatedCase {
    pub case: Case,
    #[serde(default)]
    pub answer: Option<Map<String, Value>>,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub reason: String,
}

impl EvaluatedCase {
    pub fn inputs(&self) -> &Map<String, Value> {
        &self.case.inputs
    }

    pub fn label(&self) -> &Map<String, Value> {
        &self.case.label
    }

    pub fn case_id(&self) -> &str {
        &self.case.case_id
    }
}

/// 用例加载器(对齐 CaseLoader)。
///
/// 构造时按序分配 case_{i} id;`shuffle` 用确定性伪随机(LCG 种子)打乱并
/// 重新分配 id;`split` 深拷贝 + 打乱后按比例切分。
#[derive(Debug, Clone)]
pub struct CaseLoader {
    cases: Vec<Case>,
}

impl CaseLoader {
    pub fn new(cases: Vec<Case>) -> Self {
        let mut loader = Self { cases };
        loader.assign_case_id();
        loader
    }

    pub fn len(&self) -> usize {
        self.cases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    pub fn size(&self) -> usize {
        self.cases.len()
    }

    pub fn get_cases(&self) -> &[Case] {
        &self.cases
    }

    /// 确定性打乱(LCG;seed 对齐 Python `random.seed` 的可复现性由调用方
    /// 传入种子保证):打乱后重新分配 case_id。
    pub fn shuffle(&mut self, seed: u64) {
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        for i in (1..self.cases.len()).rev() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (state >> 33) as usize % (i + 1);
            self.cases.swap(i, j);
        }
        self.assign_case_id();
    }

    /// 按比例切分为两个 loader(对齐 `split`;ratio 越界回退 0.5)。
    pub fn split(&self, ratio: f64, seed: u64) -> (CaseLoader, CaseLoader) {
        let ratio = if (0.0..=1.0).contains(&ratio) {
            ratio
        } else {
            0.5
        };
        let mut shuffled = self.cases.clone();
        let mut state = seed.wrapping_add(0x9E3779B97F4A7C15);
        for i in (1..shuffled.len()).rev() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = (state >> 33) as usize % (i + 1);
            shuffled.swap(i, j);
        }
        let cut = ((self.cases.len() as f64) * ratio) as usize;
        let left = shuffled[..cut].to_vec();
        let right = shuffled[cut..].to_vec();
        (CaseLoader::new(left), CaseLoader::new(right))
    }

    fn assign_case_id(&mut self) {
        for (i, case) in self.cases.iter_mut().enumerate() {
            case.case_id = format!("case_{i}");
        }
    }
}

/// tune 工具纯函数(对齐 TuneUtils)。
pub struct TuneUtils;

impl TuneUtils {
    /// 校验数字参数在 [lower, upper] 内(对齐 `validate_digital_parameter`)。
    pub fn validate_digital_parameter(
        param: f64,
        param_name: &str,
        lower: f64,
        upper: f64,
    ) -> Result<(), TuneKitError> {
        if param < lower || param > upper {
            Err(TuneKitError(format!(
                "{param_name} should be between {lower} and {upper}"
            )))
        } else {
            Ok(())
        }
    }

    /// 从 LLM 响应提取 ```json ... ``` 块并解析为 JSON(对齐
    /// `parse_json_from_llm_response`;无匹配/解析失败 → None)。
    pub fn parse_json_from_llm_response(json_like: &str) -> Option<Value> {
        let start = json_like.find("```json")?;
        let after = &json_like[start + "```json".len()..];
        let end = after.find("```")?;
        let body = after[..end].trim();
        serde_json::from_str(body).ok()
    }

    /// 从 LLM 响应提取 ```list ... ``` 块并解析为值列表(对齐
    /// `parse_list_from_llm_response` 的 JSON 形态;无匹配 → None)。
    pub fn parse_list_from_llm_response(list_like: &str) -> Option<Vec<Value>> {
        let start = list_like.find("```list")?;
        let after = &list_like[start + "```list".len()..];
        let end = after.find("```")?;
        let body = after[..end].trim();
        let parsed: Value = serde_json::from_str(body).ok()?;
        parsed.as_array().cloned()
    }

    /// 把用例列表格式化为 few-shot examples 文本(对齐
    /// `convert_cases_to_examples`)。
    pub fn convert_cases_to_examples(cases: &[EvaluatedCase]) -> String {
        if cases.is_empty() {
            return String::new();
        }
        let lines: Vec<String> = cases
            .iter()
            .enumerate()
            .map(|(i, case)| {
                format!(
                    "example {}:\n[question]: {}\n[expected answer]: {}",
                    i + 1,
                    Self::convert_dict_to_string(case.inputs()),
                    Self::convert_dict_to_string(case.label())
                )
            })
            .collect();
        lines.join("\n")
    }

    /// dict → "key:value | ..."(对齐 `_convert_dict_to_string`)。
    pub fn convert_dict_to_string(data: &Map<String, Value>) -> String {
        data.iter()
            .map(|(k, v)| format!("{k}:{v}"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

/// 文本参数(对齐 TextualParameter)。
#[derive(Debug, Clone, Default)]
pub struct TextualParameter {
    pub gradients: HashMap<String, String>,
    pub description: String,
}

impl TextualParameter {
    pub fn set_gradient(&mut self, name: &str, gradient: &str) {
        self.gradients
            .insert(name.to_string(), gradient.to_string());
    }

    pub fn get_gradient(&self, name: &str) -> Option<&str> {
        self.gradients.get(name).map(String::as_str)
    }

    pub fn set_description(&mut self, description: &str) {
        self.description = description.to_string();
    }

    pub fn get_description(&self) -> &str {
        &self.description
    }
}

/// 追踪节点(对齐 TraceNode 的确定性字段)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TraceNode {
    pub case_id: String,
    pub llm_call_id: String,
    pub inputs: Map<String, Value>,
    pub outputs: String,
}

/// 优化历史(对齐 OptimizeHistory;case_id → TraceNode 列表)。
#[derive(Debug, Clone, Default)]
pub struct OptimizeHistory {
    trajectory: HashMap<String, Vec<TraceNode>>,
}

impl OptimizeHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_history(&mut self, case_id: &str, node: TraceNode) {
        self.trajectory
            .entry(case_id.to_string())
            .or_default()
            .push(node);
    }

    pub fn get_history(&self, case_id: &str) -> Option<&[TraceNode]> {
        self.trajectory.get(case_id).map(Vec::as_slice)
    }

    pub fn get_llm_call_history(&self, case_id: &str, llm_call_id: &str) -> Vec<&TraceNode> {
        self.get_history(case_id)
            .unwrap_or(&[])
            .iter()
            .filter(|node| node.llm_call_id == llm_call_id)
            .collect()
    }

    pub fn clear_history(&mut self) {
        self.trajectory.clear();
    }
}

/// 训练进度(对齐 trainer/base.py Progress;run_epoch / run_batch 为
/// 确定性迭代器语义,由调用方驱动)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    pub current_epoch: u32,
    pub max_epoch: u32,
    pub current_batch_iter: u32,
    pub max_batch_iter: u32,
    pub best_score: f64,
    pub best_batch_score: f64,
    pub current_epoch_score: f64,
}

impl Progress {
    pub fn new() -> Self {
        Self {
            current_epoch: 0,
            max_epoch: tune_constant::DEFAULT_ITERATION_NUM,
            current_batch_iter: 0,
            max_batch_iter: 1,
            best_score: 0.0,
            best_batch_score: 0.0,
            current_epoch_score: 0.0,
        }
    }

    /// 推进一轮(epoch 从 1 起,收敛到 max_epoch;对齐 `run_epoch`)。
    pub fn run_epoch(&mut self) -> Option<u32> {
        if self.current_epoch >= self.max_epoch {
            return None;
        }
        self.current_epoch += 1;
        Some(self.current_epoch)
    }

    /// 推进一批(重置 best_batch_score;迭代 0..max_batch_iter;对齐 `run_batch`)。
    pub fn run_batch(&mut self) -> Option<u32> {
        if self.current_batch_iter >= self.max_batch_iter {
            return None;
        }
        if self.current_batch_iter == 0 {
            self.best_batch_score = 0.0;
        }
        let iter = self.current_batch_iter;
        self.current_batch_iter += 1;
        Some(iter)
    }
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 LLM 响应提取优化后的 prompt(对齐
/// `_extract_optimized_prompt_from_response`):`<tag>...</tag>` 提取 +
/// 去除 <prompt_base> 包裹)。
pub fn extract_optimized_prompt_from_response(response: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = response.find(&open)?;
    let after = &response[start + open.len()..];
    let end = after.find(&close)?;
    Some(
        after[..end]
            .replace("<prompt_base>", "")
            .replace("</prompt_base>", ""),
    )
}

/// 查找提示词中的 `{{placeholder}}` 占位符(对齐 `_find_placeholders_from_prompt`)。
pub fn find_placeholders_from_prompt(prompt: &str) -> Vec<String> {
    let mut placeholders = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        if let Some(end) = after.find("}}") {
            let name = after[..end].trim().to_string();
            if !name.is_empty() {
                placeholders.push(name);
            }
            rest = &after[end + 2..];
        } else {
            break;
        }
    }
    placeholders
}

/// 计算缺失占位符(对齐 `_find_missing_placeholders`;原集 - 优化集)。
pub fn find_missing_placeholders(original: &[String], optimized: &[String]) -> Vec<String> {
    let mut missing: Vec<String> = original
        .iter()
        .filter(|p| !optimized.contains(p))
        .cloned()
        .collect();
    missing.sort();
    missing.dedup();
    missing
}

/// 渲染 bad-case 文本(对齐 CREATE_BAD_CASE_TEMPLATE 格式化)。
pub fn create_bad_case_text(question: &str, label: &str, answer: &str, reason: &str) -> String {
    format!(
        "[question]: {question}\n[expected answer]: {label}\n[assistant answer]: {answer}\n[reason]: {reason}\n=== \n"
    )
}

/// 评估结果 → 分数(对齐 DefaultEvaluator.evaluate 的评分决策):
/// result 为 true 或字符串 "true" → 1.0,否则 0.0。
pub fn evaluate_result_to_score(result: &Value) -> f64 {
    match result {
        Value::Bool(true) => 1.0,
        Value::String(s) if s.trim().eq_ignore_ascii_case("true") => 1.0,
        _ => 0.0,
    }
}

/// tune-kit 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuneKitError(pub String);

impl core::fmt::Display for TuneKitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TuneKitError {}

/// tune-kit Seam(Service Definition):确定性训练工具门面。
pub trait TuneKit: Seam {
    /// 构建用例加载器(分配 case_id)。
    fn case_loader(&self, cases: Vec<Case>) -> CaseLoader;

    /// 校验数字参数范围。
    fn validate_digital_parameter(
        &self,
        param: f64,
        param_name: &str,
        lower: f64,
        upper: f64,
    ) -> Result<(), TuneKitError>;

    /// 解析 ```json``` 块。
    fn parse_json_from_llm_response(&self, json_like: &str) -> Option<Value>;

    /// 解析 ```list``` 块。
    fn parse_list_from_llm_response(&self, list_like: &str) -> Option<Vec<Value>>;

    /// 用例 → few-shot examples 文本。
    fn convert_cases_to_examples(&self, cases: &[EvaluatedCase]) -> String;

    /// 提取优化 prompt 标签。
    fn extract_optimized_prompt(&self, response: &str, tag: &str) -> Option<String>;

    /// 查找占位符。
    fn find_placeholders(&self, prompt: &str) -> Vec<String>;

    /// 计算缺失占位符。
    fn missing_placeholders(&self, original: &[String], optimized: &[String]) -> Vec<String>;

    /// 评估结果 → 分数。
    fn score_from_result(&self, result: &Value) -> f64;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn case(inputs: Map<String, Value>, label: Map<String, Value>) -> Case {
        Case::new(inputs, label)
    }

    #[test]
    fn case_loader_assigns_and_shuffles() {
        let cases = vec![
            case(Map::from_iter([("q".into(), json!("a"))]), Map::new()),
            case(Map::from_iter([("q".into(), json!("b"))]), Map::new()),
            case(Map::from_iter([("q".into(), json!("c"))]), Map::new()),
            case(Map::from_iter([("q".into(), json!("d"))]), Map::new()),
        ];
        let loader = CaseLoader::new(cases);
        assert_eq!(loader.size(), 4);
        assert_eq!(loader.get_cases()[0].case_id, "case_0");
        assert_eq!(loader.get_cases()[3].case_id, "case_3");
        // shuffle 后 id 重新按序分配。
        let mut loader = loader;
        loader.shuffle(42);
        assert_eq!(loader.get_cases()[0].case_id, "case_0");
        assert_eq!(loader.len(), 4);
        // 同种子确定性。
        let mut a = loader.clone();
        let mut b = loader.clone();
        a.shuffle(7);
        b.shuffle(7);
        assert_eq!(a.get_cases(), b.get_cases());
    }

    #[test]
    fn case_loader_split() {
        let cases = (0..10)
            .map(|i| case(Map::from_iter([("i".into(), json!(i))]), Map::new()))
            .collect();
        let loader = CaseLoader::new(cases);
        let (left, right) = loader.split(0.5, 99);
        assert_eq!(left.size(), 5);
        assert_eq!(right.size(), 5);
        // ratio 越界回退 0.5。
        let (l2, r2) = loader.split(2.0, 99);
        assert_eq!(l2.size() + r2.size(), 10);
    }

    #[test]
    fn validate_parameter_range() {
        assert!(TuneUtils::validate_digital_parameter(5.0, "num_parallel", 1.0, 20.0).is_ok());
        let err =
            TuneUtils::validate_digital_parameter(0.0, "num_parallel", 1.0, 20.0).unwrap_err();
        assert!(
            err.0.contains("num_parallel should be between 1 and 20"),
            "{err}"
        );
    }

    #[test]
    fn parse_json_and_list_blocks() {
        let json_resp = "前缀\n```json\n{\"result\": true, \"reason\": \"ok\"}\n```\n后缀";
        let parsed = TuneUtils::parse_json_from_llm_response(json_resp).expect("json");
        assert_eq!(parsed["result"], true);
        assert!(TuneUtils::parse_json_from_llm_response("no block").is_none());
        assert!(TuneUtils::parse_json_from_llm_response("```json\nnot-json\n```").is_none());

        let list_resp = "```list\n[0, 2, 4]\n```";
        let parsed_list = TuneUtils::parse_list_from_llm_response(list_resp).expect("list");
        assert_eq!(parsed_list.len(), 3);
        // 非列表 JSON(如对象)→ None。
        assert!(TuneUtils::parse_list_from_llm_response("```list\n{\"a\": 1}\n```").is_none());
        // 无块 → None。
        assert!(TuneUtils::parse_list_from_llm_response("plain").is_none());
    }

    #[test]
    fn examples_and_dict_formatting() {
        let evaluated = EvaluatedCase {
            case: case(
                Map::from_iter([("q".into(), json!("what"))]),
                Map::from_iter([("a".into(), json!("42"))]),
            ),
            answer: None,
            score: 1.0,
            reason: String::new(),
        };
        let text = TuneUtils::convert_cases_to_examples(&[evaluated]);
        assert!(text.contains("example 1:"), "{text}");
        assert!(text.contains("[question]: q:\"what\""), "{text}");
        assert!(text.contains("[expected answer]: a:\"42\""), "{text}");
        assert_eq!(TuneUtils::convert_cases_to_examples(&[]), "");
    }

    #[test]
    fn optimizer_history_and_textual_parameter() {
        let mut history = OptimizeHistory::new();
        history.add_history(
            "c1",
            TraceNode {
                case_id: "c1".to_string(),
                llm_call_id: "n1".to_string(),
                inputs: Map::new(),
                outputs: "out1".to_string(),
            },
        );
        history.add_history(
            "c1",
            TraceNode {
                case_id: "c1".to_string(),
                llm_call_id: "n2".to_string(),
                inputs: Map::new(),
                outputs: "out2".to_string(),
            },
        );
        assert_eq!(history.get_history("c1").map(|h| h.len()), Some(2));
        assert_eq!(history.get_llm_call_history("c1", "n1").len(), 1);
        assert_eq!(history.get_llm_call_history("c1", "nope").len(), 0);
        assert!(history.get_history("missing").is_none());
        history.clear_history();
        assert!(history.get_history("c1").is_none());

        let mut param = TextualParameter::default();
        param.set_gradient("system_prompt", "g1");
        assert_eq!(param.get_gradient("system_prompt"), Some("g1"));
        param.set_description("desc");
        assert_eq!(param.get_description(), "desc");
    }

    #[test]
    fn progress_run_epoch_and_batch() {
        let mut progress = Progress::new();
        progress.max_epoch = 2;
        progress.max_batch_iter = 2;
        assert_eq!(progress.run_epoch(), Some(1));
        assert_eq!(progress.run_epoch(), Some(2));
        assert_eq!(progress.run_epoch(), None, "超上限停");
        // run_batch:重置 best_batch_score 后迭代。
        progress.best_batch_score = 0.8;
        assert_eq!(progress.run_batch(), Some(0));
        assert_eq!(progress.best_batch_score, 0.0, "批首重置");
        assert_eq!(progress.run_batch(), Some(1));
        assert_eq!(progress.run_batch(), None);
    }

    #[test]
    fn extract_optimized_prompt_tags() {
        let response = "分析\n<PROMPT_OPTIMIZED>\n优化后的提示词\n</PROMPT_OPTIMIZED>\n结尾";
        let extracted =
            extract_optimized_prompt_from_response(response, "PROMPT_OPTIMIZED").expect("tag");
        // Python 用 regex `(.*?)` DOTALL 捕获,不 strip —— 保留标签间原始内容(含换行)。
        assert_eq!(extracted, "\n优化后的提示词\n");
        // 无标签 → None。
        assert!(extract_optimized_prompt_from_response("plain", "X").is_none());
        // 去除 prompt_base 包裹(不 strip,对齐 Python `replace`)。
        let wrapped = "<PROMPT_OPTIMIZED>\n<prompt_base>内容</prompt_base>\n</PROMPT_OPTIMIZED>";
        assert_eq!(
            extract_optimized_prompt_from_response(wrapped, "PROMPT_OPTIMIZED").unwrap(),
            "\n内容\n"
        );
    }

    #[test]
    fn placeholders_find_and_missing() {
        let prompt = "Task {{task.title}} by {{task.assignee}} and {{param.x}}";
        let found = find_placeholders_from_prompt(prompt);
        assert_eq!(found, vec!["task.title", "task.assignee", "param.x"]);
        // 优化后缺一个。
        let optimized = find_placeholders_from_prompt("Task {{task.title}}");
        let missing = find_missing_placeholders(&found, &optimized);
        assert_eq!(missing, vec!["param.x", "task.assignee"]);
        // 无占位符。
        assert!(find_placeholders_from_prompt("plain").is_empty());
    }

    #[test]
    fn bad_case_text_and_score() {
        let text = create_bad_case_text("q", "l", "a", "r");
        assert!(text.contains("[question]: q"));
        assert!(text.contains("[expected answer]: l"));
        assert!(text.contains("[assistant answer]: a"));
        assert!(text.contains("[reason]: r"));
        assert!(text.contains("=== "));
        assert_eq!(evaluate_result_to_score(&json!(true)), 1.0);
        assert_eq!(evaluate_result_to_score(&json!("true")), 1.0);
        assert_eq!(evaluate_result_to_score(&json!("TRUE")), 1.0);
        assert_eq!(evaluate_result_to_score(&json!(false)), 0.0);
        assert_eq!(evaluate_result_to_score(&json!("false")), 0.0);
        assert_eq!(evaluate_result_to_score(&json!("n/a")), 0.0);
    }
}
