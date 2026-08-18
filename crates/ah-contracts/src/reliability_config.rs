//! reliability 配置(对齐 openjiuwen/agent_teams/reliability/config.py)。
//!
//! 纯数据契约:各检测器阈值 + 修复策略 + 重启强度预算,serde 可序列化
//! (profile/JSON 配置来源)。插件按字段构建真实检测器/修复器。

use crate::reliability_detectors::Severity;

/// 工具调用错误率检测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolErrorConfig {
    pub enabled: bool,
    pub window_seconds: f64,
    pub rate_threshold: u64,
    pub consecutive_threshold: u64,
}

impl Default for ToolErrorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_seconds: 60.0,
            rate_threshold: 5,
            consecutive_threshold: 3,
        }
    }
}

/// 重复/循环工具调用检测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RepeatToolConfig {
    pub enabled: bool,
    pub history_size: usize,
    pub repeat_warn: u64,
    pub pingpong_warn: u64,
    pub loop_block: u64,
    pub global_stop: u64,
}

impl Default for RepeatToolConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            history_size: 30,
            repeat_warn: 10,
            pingpong_warn: 10,
            loop_block: 20,
            global_stop: 30,
        }
    }
}

/// 模型调用错误检测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelErrorConfig {
    pub enabled: bool,
    pub window_seconds: f64,
    pub rate_threshold: u64,
    pub consecutive_threshold: u64,
}

impl Default for ModelErrorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_seconds: 120.0,
            rate_threshold: 3,
            consecutive_threshold: 2,
        }
    }
}

/// 超长输出/思考检测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputLengthConfig {
    pub enabled: bool,
    pub text_threshold: u64,
    pub thinking_threshold: u64,
}

impl Default for OutputLengthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            text_threshold: 32000,
            thinking_threshold: 16000,
        }
    }
}

/// 频繁压缩(上下文裁剪)推断配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompactionConfig {
    pub enabled: bool,
    pub window_seconds: f64,
    pub frequency_threshold: u64,
    pub drop_ratio: f64,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            window_seconds: 300.0,
            frequency_threshold: 3,
            drop_ratio: 0.3,
        }
    }
}

/// 团队级乒乓(双向消息往返)检测配置。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PingPongConfig {
    pub enabled: bool,
    pub min_volleys: u64,
}

impl Default for PingPongConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_volleys: 6,
        }
    }
}

/// 全部检测器的开关与阈值(对齐 DetectorsConfig)。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct DetectorsConfig {
    pub tool_error: ToolErrorConfig,
    pub repeat_tool: RepeatToolConfig,
    pub model_error: ModelErrorConfig,
    pub output_length: OutputLengthConfig,
    pub compaction: CompactionConfig,
    pub pingpong: PingPongConfig,
}

/// 本地自动修复的强度预算(对齐 RestartIntensityConfig)。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RestartIntensityConfig {
    pub intensity: u64,
    pub period_seconds: f64,
}

impl Default for RestartIntensityConfig {
    fn default() -> Self {
        Self {
            intensity: 5,
            period_seconds: 60.0,
        }
    }
}

/// 修复动作(对齐 RemediationAction)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationAction {
    ObserveOnly,
    ReportLeader,
    LocalSteer,
    EscalateUser,
}

/// 严重度→动作映射配置(对齐 RemediationPolicyConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RemediationPolicyConfig {
    pub severity_actions: std::collections::BTreeMap<Severity, Vec<RemediationAction>>,
}

impl Default for RemediationPolicyConfig {
    fn default() -> Self {
        use RemediationAction::*;
        let mut m = std::collections::BTreeMap::new();
        m.insert(Severity::Low, vec![ObserveOnly]);
        m.insert(Severity::Medium, vec![ReportLeader]);
        m.insert(Severity::High, vec![LocalSteer, ReportLeader]);
        m.insert(Severity::Critical, vec![LocalSteer, EscalateUser]);
        Self {
            severity_actions: m,
        }
    }
}

/// 团队可靠性框架总配置(对齐 ReliabilityConfig;opt-in:enabled 默认 False)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReliabilityConfig {
    pub enabled: bool,
    pub monitor_roles: Vec<String>,
    pub detectors: DetectorsConfig,
    pub policy: RemediationPolicyConfig,
    pub restart_intensity: RestartIntensityConfig,
}

impl Default for ReliabilityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            monitor_roles: vec!["leader".to_string(), "teammate".to_string()],
            detectors: DetectorsConfig::default(),
            policy: RemediationPolicyConfig::default(),
            restart_intensity: RestartIntensityConfig::default(),
        }
    }
}
