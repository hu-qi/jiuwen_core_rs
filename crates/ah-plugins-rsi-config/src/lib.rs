//! # ah-plugins-rsi-config
//!
//! Real rsi configuration models (aligned with openjiuwen/rsi/config/config.py):
//! 12 dataclass configs with from_dict parsing. Pure data + validation.
//!
//! 模块拆分:
//! - core.rs:基础配置(DataLoader/DatasetCuration/ModelConfigs/SeedEvaluation);
//! - scheduling.rs:调度配置 + 插件注册;
//! - extended.rs:扩展配置(Evaluator/DatasetGenerator/Analyzer/Skill/Experience/MemberOptimizer);
//! - auto.rs:顶层编排配置(AutoCoordinatingHarnessConfig)。

pub mod auto;
pub mod core;
pub mod extended;
pub mod scheduling;

pub use auto::AutoCoordinatingHarnessConfig;
pub use core::{DataLoaderConfig, DatasetCurationConfig, ModelConfigs, SeedEvaluationConfig};
pub use extended::{
    DatasetGeneratorConfig, EvaluationResultAnalyzerConfig, EvaluatorConfig, MemberOptimizerConfig,
    OptimizationExperienceLearnerConfig, TeamSkillOptimizerConfig,
};
pub use scheduling::OrchestratorSchedulingConfig;

pub use scheduling::{RsiConfigPlugin, RsiConfigService};
