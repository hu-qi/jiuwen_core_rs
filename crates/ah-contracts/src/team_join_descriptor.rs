//! team-join-descriptor seam:外部 agent 加入团队的连接描述。
//!
//! 对齐 openjiuwen/agent_teams/external/descriptor.py:`TeamJoinDescriptor` 把团队
//! 数据库位置、messager 可达性、成员身份打包成一个 JSON 可序列化载荷,由团队在
//! spawn 时经环境变量注入,或由运维带外下发给独立服务。
//!
//! 本文件只声明类型 + trait + 常量(契约零实现);真实 JSON/env 编解码与校验逻辑
//! 在 ah-plugins-team-join-descriptor 插件中实现。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::seam::Seam;

/// 承载 JSON 描述符的环境变量名(对齐 `OPENJIUWEN_TEAM_JOIN`)。
pub const TEAM_JOIN_ENV: &str = "OPENJIUWEN_TEAM_JOIN";

/// 外部接入场景(与团队 `role` 正交):`operator` = 团队外非成员控制面(默认),
/// `member` = 一等团队成员(真实 teammate 工具集)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Operator,
    Member,
}

/// 任务派发模式:`autonomous`(默认)或 `scheduled`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    #[default]
    Autonomous,
    Scheduled,
}

/// teammate 执行模式:`build_mode`(默认)或 `plan_mode`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TeammateMode {
    #[default]
    BuildMode,
    PlanMode,
}

/// 团队数据库连接配置(仅连接串;file-backed sqlite 用于跨进程,`:memory:` 用于测试)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct JoinDbConfig {
    #[serde(default)]
    pub connection_string: String,
}

/// Messager 传输配置:`external_publish_url` 是团队标准事件的 Gateway relay WebSocket 端点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct JoinTransportConfig {
    #[serde(default)]
    pub external_publish_url: String,
}

/// 成员 role 默认值(对齐 Python `role: str = "teammate"`)。
fn default_role() -> String {
    "teammate".to_string()
}

/// 团队运行时语言默认值(对齐 Python `language: str = "cn"`)。
fn default_language() -> String {
    "cn".to_string()
}

/// 外部 agent 加入团队的完整描述(JSON 可序列化装配蓝图)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamJoinDescriptor {
    /// 会话标识:用于事件 topic 与 per-session 动态表,必须匹配团队当前 session。
    pub session_id: String,
    /// 目标团队标识。
    pub team_name: String,
    /// 该外部 agent 服务的成员身份(团队必须已注册同名成员行)。
    pub member_name: String,
    /// 成员 role(驱动 op-surface 过滤):`teammate`(默认)或 `leader`。
    #[serde(default = "default_role")]
    pub role: String,
    /// 外部接入场景。
    #[serde(default)]
    pub scope: Scope,
    /// 团队运行时语言(`cn`(默认)/ `en`)。
    #[serde(default = "default_language")]
    pub language: String,
    /// 任务派发模式。
    #[serde(default)]
    pub dispatch_mode: DispatchMode,
    /// teammate 执行模式。
    #[serde(default)]
    pub teammate_mode: TeammateMode,
    /// 团队数据库连接。
    #[serde(default)]
    pub db_config: JoinDbConfig,
    /// Messager 传输配置。
    #[serde(default)]
    pub transport_config: JoinTransportConfig,
    /// 团队宿主解析出的共享工作空间绝对路径。
    #[serde(default)]
    pub workspace_path: Option<String>,
}

/// 描述符解析/校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescriptorError(pub String);

impl core::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DescriptorError {}

/// team-join-descriptor Seam(Service Definition)。
///
/// 实现方(插件)提供真实 JSON/env 编解码;消费方(外部接入/运维)只依赖本 trait。
pub trait TeamJoinDescriptorApi: Seam {
    /// 序列化为紧凑 JSON 字符串。
    fn to_json(&self, d: &TeamJoinDescriptor) -> String;

    /// 序列化并打包为单键环境映射 `{TEAM_JOIN_ENV: <json>}`。
    fn to_env(&self, d: &TeamJoinDescriptor) -> BTreeMap<String, String>;

    /// 从 JSON 解析;缺必填字段(session_id/team_name/member_name)或非法枚举值 → 显式 `Err`。
    #[allow(clippy::wrong_self_convention)]
    fn from_json(&self, raw: &str) -> Result<TeamJoinDescriptor, DescriptorError>;

    /// 从环境映射读取 `TEAM_JOIN_ENV`(缺键 → 显式 `Err`);`env` 为 `None` 时读真实进程环境。
    #[allow(clippy::wrong_self_convention)]
    fn from_env(
        &self,
        env: Option<&BTreeMap<String, String>>,
    ) -> Result<TeamJoinDescriptor, DescriptorError>;
}
