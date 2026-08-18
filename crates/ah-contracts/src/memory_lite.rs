//! memory-lite seam(对齐 core/memory/lite/{frontmatter,types,conflict_types}.py)。
//!
//! 纯数据契约:记忆 frontmatter 解析/校验/丰富、chunk 模型、写入模式与结果。

/// 合法记忆类型(对齐 VALID_TYPES)。
pub const VALID_TYPES: [&str; 4] = ["user", "feedback", "project", "reference"];

/// 记忆 chunk(对齐 MemoryChunk)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryChunk {
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
}

impl MemoryChunk {
    pub fn new(text: impl Into<String>, start_line: usize, end_line: usize) -> Self {
        Self {
            text: text.into(),
            start_line,
            end_line,
        }
    }
}

/// 写入模式(对齐 WriteMode)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    Create,
    Append,
    Skip,
}

/// 记忆写入结果(对齐 WriteResult)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WriteResult {
    pub success: bool,
    pub path: String,
    pub mode: WriteMode,
    pub conflict_detected: bool,
    pub conflicting_files: Vec<String>,
    pub note: Option<String>,
    pub error: Option<String>,
    pub r#type: Option<String>,
}

impl WriteResult {
    pub fn new(success: bool, path: impl Into<String>, mode: WriteMode) -> Self {
        Self {
            success,
            path: path.into(),
            mode,
            conflict_detected: false,
            conflicting_files: Vec::new(),
            note: None,
            error: None,
            r#type: None,
        }
    }

    /// 转为工具响应 dict(对齐 to_dict;仅含非默认字段)。
    pub fn to_dict(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        m.insert("success".to_string(), serde_json::Value::Bool(self.success));
        m.insert(
            "path".to_string(),
            serde_json::Value::String(self.path.clone()),
        );
        let mode_str = match self.mode {
            WriteMode::Create => "create",
            WriteMode::Append => "append",
            WriteMode::Skip => "skip",
        };
        m.insert(
            "mode".to_string(),
            serde_json::Value::String(mode_str.to_string()),
        );
        if let Some(t) = &self.r#type {
            m.insert("type".to_string(), serde_json::Value::String(t.clone()));
        }
        if self.conflict_detected {
            m.insert(
                "conflict_detected".to_string(),
                serde_json::Value::Bool(true),
            );
            m.insert(
                "conflicting_files".to_string(),
                serde_json::Value::Array(
                    self.conflicting_files
                        .iter()
                        .cloned()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(n) = &self.note {
            m.insert("note".to_string(), serde_json::Value::String(n.clone()));
        }
        if let Some(e) = &self.error {
            m.insert("error".to_string(), serde_json::Value::String(e.clone()));
        }
        serde_json::Value::Object(m)
    }
}
/// 记忆配置(对齐 MemorySettings;默认值与 Python 完全一致)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemorySettings {
    pub model: String,
    pub sources: Vec<String>,
    pub extra_paths: Vec<String>,
    pub chunking_tokens: usize,
    pub chunking_overlap: usize,
    pub query_max_results: usize,
    pub query_min_score: f64,
    pub hybrid_enabled: bool,
    pub hybrid_vector_weight: f64,
    pub hybrid_text_weight: f64,
    pub hybrid_candidate_multiplier: f64,
    pub store_path: String,
    pub vector_enabled: bool,
    pub fts_enabled: bool,
    pub sync_watch: bool,
    pub sync_watch_debounce_ms: u64,
    pub sync_on_search: bool,
    pub sync_on_session_start: bool,
    pub sync_interval_minutes: u64,
    pub cache_enabled: bool,
    pub cache_max_entries: usize,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            model: "text-embedding-v3".to_string(),
            sources: vec!["memory".to_string(), "sessions".to_string()],
            extra_paths: Vec::new(),
            chunking_tokens: 256,
            chunking_overlap: 32,
            query_max_results: 10,
            query_min_score: 0.3,
            hybrid_enabled: true,
            hybrid_vector_weight: 0.7,
            hybrid_text_weight: 0.3,
            hybrid_candidate_multiplier: 2.0,
            store_path: "memory.db".to_string(),
            vector_enabled: true,
            fts_enabled: true,
            sync_watch: true,
            sync_watch_debounce_ms: 2000,
            sync_on_search: true,
            sync_on_session_start: true,
            sync_interval_minutes: 0,
            cache_enabled: true,
            cache_max_entries: 10000,
        }
    }
}

impl MemorySettings {
    /// 应用覆盖(对齐 create_memory_settings 的 **overrides;未知键忽略)。
    pub fn with_overrides(&self, overrides: &[(&str, serde_json::Value)]) -> Self {
        let mut out = self.clone();
        for (key, value) in overrides {
            match *key {
                "model" => {
                    if let Some(v) = value.as_str() {
                        out.model = v.to_string();
                    }
                }
                "sources" => {
                    if let Some(v) = value.as_array() {
                        out.sources = v
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect();
                    }
                }
                "extra_paths" => {
                    if let Some(v) = value.as_array() {
                        out.extra_paths = v
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect();
                    }
                }
                _ => {}
            }
        }
        out
    }
}
