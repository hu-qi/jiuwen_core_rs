//! 自进化数据集类型与容器(对齐 agent_evolving/dataset 包)。
//!
//! 纯逻辑:Case/EvaluatedCase(score 夹取 [0,1])+ 可复现 shuffle + 按比例 split + CaseLoader 容器。

use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::tool_metadata::unique_hex;

/// 工具信息(对齐 ToolInfo:type/name/description/parameters)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolInfo {
    #[serde(default = "default_function_type")]
    pub r#type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: Value,
}

fn default_function_type() -> String {
    "function".to_string()
}

impl ToolInfo {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            r#type: "function".to_string(),
            name: name.into(),
            description: description.into(),
            parameters: Value::Object(Map::new()),
        }
    }
}

/// 单个训练/评估样本(对齐 Case)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Case {
    pub inputs: BTreeMap<String, Value>,
    pub label: BTreeMap<String, Value>,
    pub tools: Option<Vec<ToolInfo>>,
    pub case_id: String,
}

impl Case {
    /// 自动生成 case_id(对齐 uuid4().hex 语义:唯一 hex 标识)。
    pub fn new(
        inputs: BTreeMap<String, Value>,
        label: BTreeMap<String, Value>,
        tools: Option<Vec<ToolInfo>>,
    ) -> Self {
        Self {
            inputs,
            label,
            tools,
            case_id: unique_hex(),
        }
    }
}

/// 已评估样本(score 夹取 [0,1];对齐 EvaluatedCase)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EvaluatedCase {
    pub case: Case,
    pub answer: Option<Value>,
    pub score: f64,
    #[serde(default)]
    pub reason: String,
    pub per_metric: Option<BTreeMap<String, f64>>,
}

impl EvaluatedCase {
    pub fn new(
        case: Case,
        answer: Option<Value>,
        score: f64,
        reason: impl Into<String>,
        per_metric: Option<BTreeMap<String, f64>>,
    ) -> Self {
        Self {
            case,
            answer,
            score: clamp_score(score),
            reason: reason.into(),
            per_metric,
        }
    }

    pub fn inputs(&self) -> &BTreeMap<String, Value> {
        &self.case.inputs
    }

    pub fn label(&self) -> &BTreeMap<String, Value> {
        &self.case.label
    }

    pub fn tools(&self) -> Option<&Vec<ToolInfo>> {
        self.case.tools.as_ref()
    }

    pub fn case_id(&self) -> &str {
        &self.case.case_id
    }
}

/// score 夹取到 [0,1](对齐 clamp_score)。
pub fn clamp_score(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// 确定性洗牌(seed 固定则结果固定;原列表不变,对齐 shuffle_cases)。
pub fn shuffle_cases(cases: &[Case], seed: u64) -> Vec<Case> {
    let mut rng = SplitMix64::new(seed);
    let mut shuffled = cases.to_vec();
    for i in (1..shuffled.len()).rev() {
        let j = (rng.next_u64() as usize) % (i + 1);
        shuffled.swap(i, j);
    }
    shuffled
}

/// 按比例切分(ratio ∈ [0,1],否则 Err;对齐 split_cases)。
pub fn split_cases(cases: &[Case], ratio: f64) -> Result<(Vec<Case>, Vec<Case>), String> {
    if !(0.0..=1.0).contains(&ratio) {
        return Err(format!("ratio must be in [0.0, 1.0], got {ratio}"));
    }
    let cut = (cases.len() as f64 * ratio) as usize;
    Ok((cases[..cut].to_vec(), cases[cut..].to_vec()))
}

/// 简单确定性 PRNG(SplitMix64,无外部依赖)。
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// Case 列表容器(支持迭代/拷贝/切分;对齐 CaseLoader)。
#[derive(Debug, Clone, Default)]
pub struct CaseLoader {
    cases: Vec<Case>,
}

impl CaseLoader {
    pub fn new(cases: Vec<Case>) -> Self {
        Self { cases }
    }

    pub fn len(&self) -> usize {
        self.cases.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Case> {
        self.cases.iter()
    }

    /// 内部列表拷贝(对齐 get_cases)。
    pub fn get_cases(&self) -> Vec<Case> {
        self.cases.clone()
    }

    /// 按比例洗牌切分为两个 loader(对齐 split)。
    pub fn split(&self, ratio: f64, seed: u64) -> Result<(CaseLoader, CaseLoader), String> {
        if !(0.0..=1.0).contains(&ratio) {
            return Err(format!("ratio must be in [0.0, 1.0], got {ratio}"));
        }
        let shuffled = shuffle_cases(&self.cases, seed);
        let cut = (shuffled.len() as f64 * ratio) as usize;
        Ok((
            CaseLoader::new(shuffled[..cut].to_vec()),
            CaseLoader::new(shuffled[cut..].to_vec()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_cases() -> Vec<Case> {
        (0..6)
            .map(|i| {
                Case::new(
                    BTreeMap::from([("q".to_string(), json!(format!("q{i}")))]),
                    BTreeMap::from([("a".to_string(), json!(format!("a{i}")))]),
                    None,
                )
            })
            .collect()
    }

    #[test]
    fn case_auto_id_and_tool_info_defaults() {
        let c = Case::new(
            BTreeMap::from([("q".to_string(), json!("hi"))]),
            BTreeMap::from([("a".to_string(), json!("yo"))]),
            None,
        );
        assert!(!c.case_id.is_empty());
        let t = ToolInfo::new("t1", "tool one");
        assert_eq!(t.r#type, "function");
        assert_eq!(t.name, "t1");
        assert!(t.parameters.is_object());
    }

    #[test]
    fn evaluated_case_clamps_score() {
        let c = sample_cases().remove(0);
        let high = EvaluatedCase::new(c.clone(), None, 1.5, "", None);
        assert_eq!(high.score, 1.0);
        let low = EvaluatedCase::new(c, None, -0.2, "", None);
        assert_eq!(low.score, 0.0);
    }

    #[test]
    fn evaluated_case_accessors() {
        let c = sample_cases().remove(1);
        let ec = EvaluatedCase::new(c, Some(json!("out")), 0.8, "ok", None);
        assert_eq!(ec.inputs()["q"], json!("q1"));
        assert_eq!(ec.label()["a"], json!("a1"));
        assert!(ec.tools().is_none());
        assert_eq!(ec.case_id(), ec.case.case_id);
        assert_eq!(ec.reason, "ok");
    }

    #[test]
    fn shuffle_is_deterministic_and_permutation() {
        let cases = sample_cases();
        let a = shuffle_cases(&cases, 42);
        let b = shuffle_cases(&cases, 42);
        assert_eq!(a, b);
        let mut ids: Vec<_> = a.iter().map(|c| c.case_id.clone()).collect();
        let mut original: Vec<_> = cases.iter().map(|c| c.case_id.clone()).collect();
        ids.sort();
        original.sort();
        assert_eq!(ids, original);
    }

    #[test]
    fn split_cases_cuts_by_ratio() {
        let cases = sample_cases();
        let (first, second) = split_cases(&cases, 0.5).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(second.len(), 3);
        let (all, none) = split_cases(&cases, 1.0).unwrap();
        assert_eq!(all.len(), 6);
        assert!(none.is_empty());
    }

    #[test]
    fn split_cases_rejects_bad_ratio() {
        let cases = sample_cases();
        let err = split_cases(&cases, 1.5).unwrap_err();
        assert_eq!(err, "ratio must be in [0.0, 1.0], got 1.5");
        assert!(split_cases(&cases, -0.1).is_err());
    }

    #[test]
    fn case_loader_iter_and_get_cases() {
        let loader = CaseLoader::new(sample_cases());
        assert_eq!(loader.len(), 6);
        assert_eq!(loader.iter().count(), 6);
        let copy = loader.get_cases();
        assert_eq!(copy.len(), 6);
    }

    #[test]
    fn case_loader_split_is_reproducible() {
        let loader = CaseLoader::new(sample_cases());
        let (l1, r1) = loader.split(0.5, 7).unwrap();
        let (l2, r2) = loader.split(0.5, 7).unwrap();
        assert_eq!(l1.get_cases(), l2.get_cases());
        assert_eq!(r1.get_cases(), r2.get_cases());
        assert_eq!(l1.len() + r1.len(), 6);
        assert!(loader.split(2.0, 0).is_err());
    }
}
