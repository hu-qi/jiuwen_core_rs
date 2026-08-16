//! dataset-curator seam:从评估产物挖掘回放数据集(对齐 openjiuwen/rsi/dataset_curator/curator.py)。
//!
//! - 输入:eval_ref 产物(YAML/JSON,含 cases 列表;每 case 引用 case_path/result_path/trace_path);
//! - 决策:原用例缺失/结果不确定/分数过线(未失败)/非可判题 → 拒绝;失败且可判题 → 接受;
//! - 产出:回放数据集 JSON(case_id 前缀 replay_ + provenance 元数据)、定向种子任务 JSON、
//!   curation 报告 YAML(状态/汇总/接受/拒绝明细)。
//!
//! 契约零实现:读取/判定/落盘由插件提供(如 ah-plugins-dataset-curator)。

use crate::seam::Seam;

/// 数据集策展配置(对齐 DatasetCurationConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetCurationConfig {
    pub enabled: bool,
    pub score_threshold: f64,
    pub require_judgeable_reference: bool,
    pub output_filename: String,
    pub report_filename: String,
    pub targeted_seed_filename: String,
    pub source_label: String,
}

impl Default for DatasetCurationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            score_threshold: 1.0,
            require_judgeable_reference: true,
            output_filename: "replay_cases.json".to_string(),
            report_filename: "curation_report.yaml".to_string(),
            targeted_seed_filename: "targeted_dataset_seed.json".to_string(),
            source_label: "trace_replay".to_string(),
        }
    }
}

/// 策展产物(对齐 DatasetCurationArtifact)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetCurationArtifact {
    pub status: String,
    pub eval_ref_path: String,
    pub output_dir: String,
    pub dataset_file: String,
    pub targeted_seed_file: String,
    pub report_path: String,
    pub accepted_cases: usize,
    pub rejected_cases: usize,
}

/// 策展错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurationError(pub String);

impl core::fmt::Display for CurationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CurationError {}

/// 数据集策展 Seam(Service Definition)。
pub trait DatasetCurator: Seam {
    /// 从 eval_ref 产物创建回放数据集与报告。
    fn curate(
        &self,
        config: &DatasetCurationConfig,
        eval_ref_path: &str,
        output_dir: &str,
    ) -> Result<DatasetCurationArtifact, CurationError>;
}
