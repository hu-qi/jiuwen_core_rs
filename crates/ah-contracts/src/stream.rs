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

/// 给 chunk 打上产生成员标记(对齐 stream_controller._tag_chunk 决策逻辑)。
///
/// 纯函数:
/// - 无 member_name 或非 OutputSchema → 原样(返回 None 表示透传);
/// - 已是 TeamOutputSchema 且 source_member/role 匹配 → 原样;
/// - 已是 TeamOutputSchema 但标记不同 → 拷贝更新标记;
/// - 普通 OutputSchema → 升级为 TeamOutputSchema。
#[derive(Debug, Clone, PartialEq)]
pub enum TaggedChunk {
    /// 已是 TeamOutputSchema(原样或更新标记后)。
    Team(TeamOutputSchema),
    /// 透传原 chunk(无 member 或非 OutputSchema)。
    Passthrough(serde_json::Value),
}

/// 执行 _tag_chunk 决策;`chunk` 以 JSON 表示(OutputSchema 形状)。
pub fn tag_chunk(
    chunk: &serde_json::Value,
    member_name: Option<&str>,
    role: Option<&str>,
) -> TaggedChunk {
    let member = match member_name {
        Some(m) if !m.is_empty() => m,
        _ => return TaggedChunk::Passthrough(chunk.clone()),
    };
    // 非 OutputSchema(缺 type/index/payload 形状)→ 透传。
    if chunk.get("type").is_none() || chunk.get("index").is_none() {
        return TaggedChunk::Passthrough(chunk.clone());
    }
    let has_team_fields = chunk.get("source_member").is_some() || chunk.get("role").is_some();
    if has_team_fields {
        // 已是 TeamOutputSchema:匹配则原样,否则更新标记。
        let same_member = chunk
            .get("source_member")
            .and_then(|v| v.as_str())
            .map(|s| s == member)
            .unwrap_or(false);
        let same_role = chunk
            .get("role")
            .and_then(|v| v.as_str())
            .map(|s| role.map(|r| s == r).unwrap_or(true))
            .unwrap_or(true);
        if same_member && same_role {
            return TaggedChunk::Team(serde_json::from_value(chunk.clone()).expect("team schema"));
        }
        let mut updated = chunk.clone();
        if let Some(obj) = updated.as_object_mut() {
            obj.insert(
                "source_member".to_string(),
                serde_json::Value::String(member.to_string()),
            );
            if let Some(r) = role {
                obj.insert("role".to_string(), serde_json::Value::String(r.to_string()));
            }
        }
        return TaggedChunk::Team(serde_json::from_value(updated).expect("team schema"));
    }
    // 普通 OutputSchema → 升级。
    TaggedChunk::Team(TeamOutputSchema {
        r#type: chunk["type"].as_str().unwrap_or("").to_string(),
        index: chunk["index"].as_u64().unwrap_or(0),
        payload: chunk
            .get("payload")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        source_member: Some(member.to_string()),
        role: role.map(str::to_string),
    })
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

    #[test]
    fn tag_chunk_upgrades_plain_output() {
        let chunk = serde_json::json!({"type": "llm_output", "index": 0, "payload": "text"});
        let tagged = tag_chunk(&chunk, Some("m1"), Some("teammate"));
        match tagged {
            TaggedChunk::Team(t) => {
                assert_eq!(t.source_member.as_deref(), Some("m1"));
                assert_eq!(t.role.as_deref(), Some("teammate"));
            }
            _ => panic!("expected team upgrade"),
        }
    }

    #[test]
    fn tag_chunk_passthrough_without_member() {
        let chunk = serde_json::json!({"type": "llm_output", "index": 0, "payload": "text"});
        let tagged = tag_chunk(&chunk, None, None);
        assert!(matches!(tagged, TaggedChunk::Passthrough(_)));
        // 非 OutputSchema 也透传。
        let raw = serde_json::json!({"weird": true});
        let tagged2 = tag_chunk(&raw, Some("m1"), None);
        assert!(matches!(tagged2, TaggedChunk::Passthrough(_)));
    }

    #[test]
    fn tag_chunk_matching_team_passthrough() {
        let chunk = serde_json::json!({
            "type": "answer", "index": 1, "payload": "x",
            "source_member": "m1", "role": "leader"
        });
        // 匹配 → 原样。
        let tagged = tag_chunk(&chunk, Some("m1"), Some("leader"));
        assert!(matches!(tagged, TaggedChunk::Team(_)));
        // 不匹配成员 → 更新。
        let tagged2 = tag_chunk(&chunk, Some("m2"), Some("leader"));
        if let TaggedChunk::Team(t) = tagged2 {
            assert_eq!(t.source_member.as_deref(), Some("m2"));
        } else {
            panic!("expected team update");
        }
    }
}
