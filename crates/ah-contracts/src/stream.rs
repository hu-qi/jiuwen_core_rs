//! session stream schema(对齐 core/session/stream/base.py)。
//!
//! 纯数据契约:流式输出的类型化 chunk 模型。
//! - StreamMode:输出模式(output/trace/custom);
//! - OutputSchema:type/index/payload 标准 chunk;
//! - TraceSchema:type/payload 追踪 chunk;
//! - CustomSchema:自定义 chunk(任意字段,宽松);
//! - TeamOutputSchema:团队层扩展(来源成员 + 角色)。

/// 流模式(对齐 StreamMode / BaseStreamMode)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    Output,
    Trace,
    Custom,
}

impl StreamMode {
    /// 模式字符串(对齐 .mode 属性)。
    pub fn as_str(self) -> &'static str {
        match self {
            StreamMode::Output => "output",
            StreamMode::Trace => "trace",
            StreamMode::Custom => "custom",
        }
    }
}

/// 标准输出 chunk(对齐 OutputSchema:type/index/payload)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputSchema {
    pub r#type: String,
    pub index: u64,
    pub payload: serde_json::Value,
}

impl OutputSchema {
    pub fn new(r#type: impl Into<String>, index: u64, payload: serde_json::Value) -> Self {
        Self {
            r#type: r#type.into(),
            index,
            payload,
        }
    }
}

/// 追踪 chunk(对齐 TraceSchema:type/payload)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TraceSchema {
    pub r#type: String,
    pub payload: serde_json::Value,
}

impl TraceSchema {
    pub fn new(r#type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            r#type: r#type.into(),
            payload,
        }
    }
}

/// 自定义 chunk(对齐 CustomSchema:任意字段宽松)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CustomSchema {
    pub r#type: String,
    pub payload: serde_json::Value,
}

impl CustomSchema {
    pub fn new(r#type: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            r#type: r#type.into(),
            payload,
        }
    }
}

/// 团队层输出 chunk(对齐 TeamOutputSchema:OutputSchema + 来源成员/角色)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TeamOutputSchema {
    pub r#type: String,
    pub index: u64,
    pub payload: serde_json::Value,
    pub source_member: Option<String>,
    pub role: Option<String>,
}

impl TeamOutputSchema {
    /// 从 OutputSchema 构建(对齐 from_output;原 schema 不被修改)。
    pub fn from_output(
        base: &OutputSchema,
        source_member: Option<String>,
        role: Option<String>,
    ) -> Self {
        Self {
            r#type: base.r#type.clone(),
            index: base.index,
            payload: base.payload.clone(),
            source_member,
            role,
        }
    }
}
/// stream 错误(对齐 session/stream 的错误语义;code 为稳定错误码)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamError {
    pub code: &'static str,
    pub message: String,
}

impl StreamError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl core::fmt::Display for StreamError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for StreamError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_mode_str_values() {
        assert_eq!(StreamMode::Output.as_str(), "output");
        assert_eq!(StreamMode::Trace.as_str(), "trace");
        assert_eq!(StreamMode::Custom.as_str(), "custom");
        assert_eq!(
            serde_json::to_string(&StreamMode::Output).unwrap(),
            "\"output\""
        );
    }

    #[test]
    fn output_schema_roundtrip() {
        let s = OutputSchema::new("nodeA", 1, serde_json::json!({"k": "v"}));
        let js = serde_json::to_string(&s).unwrap();
        let back: OutputSchema = serde_json::from_str(&js).unwrap();
        assert_eq!(back.r#type, "nodeA");
        assert_eq!(back.index, 1);
        assert_eq!(back.payload, serde_json::json!({"k": "v"}));
    }

    #[test]
    fn trace_schema_roundtrip() {
        let s = TraceSchema::new("trace-x", serde_json::json!(42));
        let js = serde_json::to_string(&s).unwrap();
        let back: TraceSchema = serde_json::from_str(&js).unwrap();
        assert_eq!(back.r#type, "trace-x");
        assert_eq!(back.payload, serde_json::json!(42));
    }

    #[test]
    fn team_output_from_output_keeps_base() {
        let base = OutputSchema::new("nodeA", 3, serde_json::json!("payload"));
        let team = TeamOutputSchema::from_output(
            &base,
            Some("member1".to_string()),
            Some("teammate".to_string()),
        );
        assert_eq!(team.r#type, "nodeA");
        assert_eq!(team.index, 3);
        assert_eq!(team.payload, serde_json::json!("payload"));
        assert_eq!(team.source_member.as_deref(), Some("member1"));
        assert_eq!(team.role.as_deref(), Some("teammate"));
        // base unchanged
        assert_eq!(base.r#type, "nodeA");
    }

    #[test]
    fn team_output_defaults_none() {
        let base = OutputSchema::new("nodeA", 0, serde_json::json!(null));
        let team = TeamOutputSchema::from_output(&base, None, None);
        assert!(team.source_member.is_none());
        assert!(team.role.is_none());
    }
}
