//! resources seam:插件 / agent-template 扩展包的加载与解析契约。
//!
//! 对齐 `openjiuwen/harness/resources/{extension_loader,extension_resolver}.py` +
//! `schema/extension_spec.py` 的确定性部分:
//! - Spec 模型:`McpServerSpec` / `SkillSpec` / `PromptSectionSpec` /
//!   `AgentTemplateSpec` / `PluginSpec`(字段、默认值、纯数据校验);
//! - 归一化纯函数:`normalize_mcp_server_entry`(transport 别名 / enabled 真值 /
//!   字段弹出)、`skill_mode` / `skill_enabled_list`、`plain_data` 校验;
//! - 模板渲染:`render_template` / `render_params` / `select_content`;
//! - 路径校验:`validate_plugin_paths`(绝对性 + 包内约束)。
//!
//! manifest 发现 / 包加载(文件系统)与运行时物化(build)由插件提供;契约只定义
//! 纯类型与判定逻辑。

use std::collections::BTreeMap;

use crate::seam::Seam;

/// resources 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcesError(pub String);

impl core::fmt::Display for ResourcesError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ResourcesError {}

/// MCP transport 别名(对齐 `_MCP_TRANSPORT_ALIASES`)。
pub fn mcp_transport_alias(transport: &str) -> Option<&'static str> {
    match transport {
        "stdio" => Some("stdio"),
        "sse" => Some("sse"),
        "http" => Some("streamable_http"),
        "streamable-http" => Some("streamable_http"),
        "streamable_http" => Some("streamable_http"),
        _ => None,
    }
}

/// 纯数据校验(对齐 `_plain_data`):None/bool/int/float/str/list/dict(str 键)。
pub fn validate_plain_data(value: &serde_json::Value) -> Result<(), ResourcesError> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => Ok(()),
        serde_json::Value::Array(items) => {
            for item in items {
                validate_plain_data(item)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => {
            for (_key, val) in map {
                validate_plain_data(val)?;
            }
            Ok(())
        }
    }
}

/// MCP server spec(对齐 `McpServerSpec`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpServerSpec {
    #[serde(default = "default_stdio")]
    pub r#type: String,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub auth_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub auth_query_params: BTreeMap<String, String>,
}

fn default_stdio() -> String {
    "stdio".to_string()
}

impl Default for McpServerSpec {
    fn default() -> Self {
        Self {
            r#type: "stdio".to_string(),
            server_name: None,
            server_id: None,
            url: None,
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            params: serde_json::Value::Object(Default::default()),
            auth_headers: BTreeMap::new(),
            auth_query_params: BTreeMap::new(),
        }
    }
}

/// Skill spec(对齐 `SkillSpec`):dir 必填,mode ∈ {all, auto_list}。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillSpec {
    pub dir: String,
    #[serde(default = "default_skill_mode")]
    pub mode: String,
    #[serde(default)]
    pub enabled_skills: Option<Vec<String>>,
}

fn default_skill_mode() -> String {
    "all".to_string()
}

/// 校验 skill mode(对齐 `_skill_mode`)。
pub fn skill_mode(value: &str) -> Result<String, ResourcesError> {
    if value == "all" || value == "auto_list" {
        Ok(value.to_string())
    } else {
        Err(ResourcesError(format!(
            "skill mode must be 'all' or 'auto_list', got {value:?}"
        )))
    }
}

/// 校验 enabled_skills(对齐 `_skill_enabled_list`)。
pub fn skill_enabled_list(
    value: Option<&serde_json::Value>,
) -> Result<Option<Vec<String>>, ResourcesError> {
    match value {
        None => Ok(None),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                out.push(match item {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
            }
            Ok(Some(out))
        }
        Some(other) => Err(ResourcesError(format!(
            "enabled_skills must be a list, got {other:?}"
        ))),
    }
}

/// Prompt section spec(对齐 `PromptSectionSpec`):name/content 必填,priority 默认 100。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PromptSectionSpec {
    pub name: String,
    pub content: BTreeMap<String, String>,
    #[serde(default = "default_section_priority")]
    pub priority: i64,
    #[serde(default)]
    pub render_params: serde_json::Value,
}

fn default_section_priority() -> i64 {
    100
}

/// 工具 spec(对齐 `BuiltinToolSpec` 的声明面)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BuiltinToolSpec {
    pub r#type: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// rail spec(对齐 `RailSpec` 的声明面)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RailSpec {
    pub r#type: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Agent template spec(对齐 `AgentTemplateSpec`)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AgentTemplateSpec {
    pub agent_card: Option<serde_json::Value>,
    pub model: Option<serde_json::Value>,
    #[serde(default)]
    pub prompt_sections: Vec<PromptSectionSpec>,
    #[serde(default)]
    pub tools: Vec<BuiltinToolSpec>,
    #[serde(default)]
    pub mcps: Vec<McpServerSpec>,
    #[serde(default)]
    pub rails: Vec<RailSpec>,
    #[serde(default)]
    pub skills: Vec<SkillSpec>,
    #[serde(default)]
    pub memories: Vec<serde_json::Value>,
    #[serde(default)]
    pub rubrics: Vec<serde_json::Value>,
    #[serde(default)]
    pub subagents: Vec<AgentTemplateSpec>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Plugin spec(对齐 `PluginSpec`):id 必填。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PluginSpec {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt_sections: Vec<PromptSectionSpec>,
    #[serde(default)]
    pub tools: Vec<BuiltinToolSpec>,
    #[serde(default)]
    pub mcps: Vec<McpServerSpec>,
    #[serde(default)]
    pub rails: Vec<RailSpec>,
    #[serde(default)]
    pub skills: Vec<SkillSpec>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// MCP server 条目归一化(对齐 `_normalize_mcp_server_entry`,纯函数无 FS)。
///
/// 返回 None 表示条目被禁用。错误消息逐字对齐。
pub fn normalize_mcp_server_entry(
    item: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<McpServerSpec>, ResourcesError> {
    let mut raw = item.clone();
    // enabled 真值语义:键存在且 falsy → 禁用。
    if let Some(enabled) = raw.get("enabled") {
        let is_falsy = match enabled {
            serde_json::Value::Bool(false) => true,
            serde_json::Value::Null => true,
            serde_json::Value::Number(n) => n.as_f64() == Some(0.0),
            serde_json::Value::String(s) => s.is_empty(),
            serde_json::Value::Array(a) => a.is_empty(),
            _ => false,
        };
        if is_falsy {
            raw.remove("enabled");
            return Ok(None);
        }
        raw.remove("enabled");
    }

    // transport = pop("transport") or pop("type") or "stdio";strip + lower。
    let transport_raw = raw
        .remove("transport")
        .or_else(|| raw.remove("type"))
        .unwrap_or(serde_json::Value::String("stdio".to_string()));
    let transport = transport_raw.as_str().unwrap_or("").trim().to_lowercase();
    let transport = if transport.is_empty() {
        "stdio".to_string()
    } else {
        transport
    };
    let client_type = mcp_transport_alias(&transport)
        .ok_or_else(|| ResourcesError(format!("Unsupported MCP transport/type: {transport:?}")))?;

    // 字段弹出与默认。
    let server_name = raw
        .remove("server_name")
        .or_else(|| raw.remove("name"))
        .and_then(|v| v.as_str().map(str::to_string))
        .filter(|s| !s.is_empty())
        .or_else(|| {
            raw.get("server_name")
                .or_else(|| raw.get("name"))
                .and_then(|v| v.as_str().map(str::to_string))
        });
    let server_id = raw
        .remove("server_id")
        .and_then(|v| v.as_str().map(str::to_string));
    let url = raw
        .remove("url")
        .and_then(|v| v.as_str().map(str::to_string));
    let command = raw
        .remove("command")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let args: Vec<String> = raw
        .remove("args")
        .and_then(|v| {
            v.as_array().map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default();
    let env: BTreeMap<String, String> = raw
        .remove("env")
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();
    let cwd = raw
        .remove("cwd")
        .and_then(|v| v.as_str().map(str::to_string));
    let params = raw
        .remove("params")
        .and_then(|v| v.as_object().map(|o| serde_json::Value::Object(o.clone())))
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let auth_headers: BTreeMap<String, String> = raw
        .remove("auth_headers")
        .or_else(|| raw.remove("headers"))
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();
    let auth_query_params: BTreeMap<String, String> = raw
        .remove("auth_query_params")
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter()
                    .filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();

    Ok(Some(McpServerSpec {
        r#type: client_type.to_string(),
        server_name,
        server_id,
        url,
        command,
        args,
        env,
        cwd,
        params,
        auth_headers,
        auth_query_params,
    }))
}

/// 是否 MCP server 条目(对齐 `_looks_like_mcp_server_entry`)。
pub fn looks_like_mcp_server_entry(item: &serde_json::Map<String, serde_json::Value>) -> bool {
    [
        "type",
        "transport",
        "server_name",
        "name",
        "command",
        "url",
        "server_id",
    ]
    .iter()
    .any(|key| item.contains_key(*key))
}

/// 模板渲染(对齐 `_render_template`):`{{ name }}` 替换,缺失保留原文。
pub fn render_template(text: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // 找闭合 }}。
            if let Some(end_rel) = find_closing(text, i + 2) {
                let inner = text[i + 2..end_rel].trim();
                if is_valid_ident(inner) {
                    if let Some(value) = params.get(inner) {
                        out.push_str(value);
                        i = end_rel + 2;
                        continue;
                    }
                    // 缺失键:保留原文(含 `{{ ` 与 ` }}` 的空白)。
                    out.push_str(&text[i..end_rel + 2]);
                    i = end_rel + 2;
                    continue;
                }
            }
        }
        // 拷贝当前字符。
        let ch = text[i..].chars().next().expect("char");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn find_closing(text: &str, from: usize) -> Option<usize> {
    let rest = &text[from..];
    rest.find("}}").map(|rel| from + rel)
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 渲染参数(对齐 `_render_params`):默认 language/workspace + 用户参数覆盖。
pub fn render_params(
    render_params: Option<&serde_json::Value>,
    language: &str,
    workspace_root: Option<&str>,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::new();
    params.insert("language".to_string(), language.to_string());
    params.insert(
        "workspace".to_string(),
        workspace_root.unwrap_or("").to_string(),
    );
    if let Some(serde_json::Value::Object(map)) = render_params {
        for (key, value) in map {
            params.insert(key.clone(), value.to_string());
        }
    }
    params
}

/// 内容选择(对齐 `_select_content`):language → en → cn → 首个 → ""。
pub fn select_content(content: &BTreeMap<String, String>, language: &str) -> String {
    if let Some(v) = content.get(language) {
        return v.clone();
    }
    if let Some(v) = content.get("en") {
        return v.clone();
    }
    if let Some(v) = content.get("cn") {
        return v.clone();
    }
    content.values().next().cloned().unwrap_or_default()
}

/// 解析后的 prompt section(对齐 `ResolvedPromptSection` + PromptSection 面)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedPromptSection {
    pub name: String,
    pub content: BTreeMap<String, String>,
    pub priority: i64,
    /// 仅 AgentTemplate 根 persona `identity` 为 True;Plugin 恒 False。
    #[serde(default)]
    pub replace_existing: bool,
}

/// 解析后的 skill(对齐 `ResolvedSkill`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedSkill {
    pub directory: String,
    pub mode: String,
    #[serde(default)]
    pub enabled_skills: Option<Vec<String>>,
}

/// 资源种类(对齐 `ResourceKind`)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Tool,
    Mcp,
    Rail,
    PromptSection,
    Skill,
    Subagent,
}

/// 资源引用(对齐 `ResourceRef`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResourceRef {
    pub kind: ResourceKind,
    pub identity: String,
    #[serde(default)]
    pub extra: serde_json::Value,
}

/// 加载记录(对齐 `LoadRecord`)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LoadRecord {
    pub load_id: String,
    #[serde(default)]
    pub source_uri: Option<String>,
    #[serde(default)]
    pub refs: Vec<ResourceRef>,
}

/// 解析后的扩展零件(对齐 `ExtensionParts`)。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExtensionParts {
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
    #[serde(default)]
    pub mcps: Vec<serde_json::Value>,
    #[serde(default)]
    pub rails: Vec<serde_json::Value>,
    #[serde(default)]
    pub prompt_sections: Vec<ResolvedPromptSection>,
    #[serde(default)]
    pub skills: Vec<ResolvedSkill>,
    #[serde(default)]
    pub subagents: Vec<serde_json::Value>,
}

/// resources Seam(Service Definition):spec → 运行时零件。
pub trait ResourcesResolver: Seam {
    /// 物化 plugin 的扩展零件(工具/rails 经 manifest 注册表构建)。
    fn resolve_plugin_parts(
        &self,
        spec: &PluginSpec,
        language: &str,
        workspace_root: Option<&str>,
    ) -> Result<ExtensionParts, ResourcesError>;

    /// 物化 agent-template 的扩展零件(含子代理,需 parent_model)。
    fn resolve_agent_template_parts(
        &self,
        spec: &AgentTemplateSpec,
        language: &str,
        workspace_root: Option<&str>,
        parent_model: Option<&serde_json::Value>,
    ) -> Result<ExtensionParts, ResourcesError>;
}

/// 路径校验(对齐 `validate_plugin_paths` 的确定性部分)。
///
/// `is_absolute(path)` 由调用方注入(避免 FS 依赖);规则:tool/rail file 型
/// params.file_path、mcp cwd、skills.dir 必须绝对,递归子代理。
pub fn validate_plugin_paths(
    tools: &[BuiltinToolSpec],
    rails: &[RailSpec],
    mcps: &[McpServerSpec],
    skills: &[SkillSpec],
    subagents: &[AgentTemplateSpec],
    is_absolute: &dyn Fn(&str) -> bool,
) -> Result<(), ResourcesError> {
    for tool in tools.iter() {
        if tool.r#type == "harness.tool.file"
            && let Some(path) = tool.params.get("file_path").and_then(|v| v.as_str())
            && !path.is_empty()
            && !is_absolute(path)
        {
            return Err(ResourcesError(format!(
                "tools.params.file_path must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {path:?}"
            )));
        }
    }
    for rail in rails.iter() {
        if rail.r#type == "harness.rail.file"
            && let Some(path) = rail.params.get("file_path").and_then(|v| v.as_str())
            && !path.is_empty()
            && !is_absolute(path)
        {
            return Err(ResourcesError(format!(
                "rails.params.file_path must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {path:?}"
            )));
        }
    }
    for mcp in mcps.iter() {
        if let Some(cwd) = &mcp.cwd
            && !cwd.is_empty()
            && !is_absolute(cwd)
        {
            return Err(ResourcesError(format!(
                "mcps.cwd must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {cwd:?}"
            )));
        }
    }
    for skill in skills.iter() {
        if !is_absolute(&skill.dir) {
            return Err(ResourcesError(format!(
                "skills.dir must already be an absolute path (the package loader or the caller of load_plugin_spec is responsible for absolutizing it): {:?}",
                skill.dir
            )));
        }
    }
    for child in subagents {
        validate_plugin_paths(
            &child.tools,
            &child.rails,
            &child.mcps,
            &child.skills,
            &child.subagents,
            is_absolute,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn transport_alias_maps() {
        assert_eq!(mcp_transport_alias("stdio"), Some("stdio"));
        assert_eq!(mcp_transport_alias("http"), Some("streamable_http"));
        assert_eq!(
            mcp_transport_alias("streamable-http"),
            Some("streamable_http")
        );
        assert_eq!(mcp_transport_alias("bogus"), None);
    }

    #[test]
    fn normalize_mcp_entry_basic() {
        let item = serde_json::json!({
            "type": "stdio",
            "server_name": "server-everything",
            "command": "npx",
            "args": ["-y", "@modelcontextprotocol/server-everything"]
        });
        let spec = normalize_mcp_server_entry(item.as_object().expect("obj"))
            .expect("ok")
            .expect("enabled");
        assert_eq!(spec.r#type, "stdio");
        assert_eq!(spec.server_name.as_deref(), Some("server-everything"));
        assert_eq!(spec.command, "npx");
        assert_eq!(
            spec.args,
            vec!["-y", "@modelcontextprotocol/server-everything"]
        );
    }

    #[test]
    fn normalize_mcp_entry_transport_alias_and_disabled() {
        let item = serde_json::json!({
            "transport": "http",
            "name": "my-server",
            "url": "http://localhost:8080",
            "enabled": false
        });
        assert!(
            normalize_mcp_server_entry(item.as_object().expect("obj"))
                .expect("ok")
                .is_none()
        );

        let item2 = serde_json::json!({"transport": "streamable-http", "name": "s", "url": "u"});
        let spec2 = normalize_mcp_server_entry(item2.as_object().expect("obj"))
            .expect("ok")
            .expect("enabled");
        assert_eq!(spec2.r#type, "streamable_http");
    }

    #[test]
    fn normalize_mcp_entry_rejects_unknown_transport() {
        let item = serde_json::json!({"transport": "weird"});
        let err = normalize_mcp_server_entry(item.as_object().expect("obj")).expect_err("bad");
        assert!(err.0.contains("Unsupported MCP transport/type"));
    }

    #[test]
    fn skill_mode_and_enabled_list() {
        assert_eq!(skill_mode("all").expect("ok"), "all");
        assert_eq!(skill_mode("auto_list").expect("ok"), "auto_list");
        assert!(skill_mode("nope").is_err());
        assert_eq!(
            skill_enabled_list(Some(&serde_json::json!(["a", "b"]))).expect("ok"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert!(skill_enabled_list(Some(&serde_json::json!("x"))).is_err());
    }

    #[test]
    fn render_template_replaces_and_keeps_missing() {
        let mut params = BTreeMap::new();
        params.insert("language".to_string(), "cn".to_string());
        let out = render_template("Hello {{ language }}!", &params);
        assert_eq!(out, "Hello cn!");
        let missing = render_template("Hi {{ nope }}", &params);
        assert_eq!(missing, "Hi {{ nope }}");
    }

    #[test]
    fn select_content_prefers_language_then_en_then_cn() {
        let mut content = BTreeMap::new();
        content.insert("cn".to_string(), "中文".to_string());
        content.insert("en".to_string(), "english".to_string());
        assert_eq!(select_content(&content, "cn"), "中文");
        assert_eq!(select_content(&content, "fr"), "english");
        let mut only_cn = BTreeMap::new();
        only_cn.insert("cn".to_string(), "中文".to_string());
        assert_eq!(select_content(&only_cn, "fr"), "中文");
    }

    #[test]
    fn validate_paths_checks_absoluteness() {
        let tools = vec![BuiltinToolSpec {
            r#type: "harness.tool.file".to_string(),
            params: serde_json::json!({"file_path": "/abs/tool.py", "class_name": "T"}),
        }];
        let ok = validate_plugin_paths(&tools, &[], &[], &[], &[], &|p| p.starts_with('/'));
        assert!(ok.is_ok());

        let rel = vec![BuiltinToolSpec {
            r#type: "harness.tool.file".to_string(),
            params: serde_json::json!({"file_path": "rel/tool.py"}),
        }];
        let err = validate_plugin_paths(&rel, &[], &[], &[], &[], &|p| p.starts_with('/'))
            .expect_err("relative");
        assert!(err.0.contains("tools.params.file_path"));

        let skill = vec![SkillSpec {
            dir: "skills/x".to_string(),
            mode: "all".to_string(),
            enabled_skills: None,
        }];
        let err2 = validate_plugin_paths(&[], &[], &[], &skill, &[], &|p| p.starts_with('/'))
            .expect_err("relative skill");
        assert!(err2.0.contains("skills.dir"));
    }

    #[test]
    fn plain_data_validation() {
        assert!(validate_plain_data(&serde_json::json!({"a": [1, "x", true, null]})).is_ok());
        assert!(validate_plain_data(&serde_json::json!([1, 2.5, false])).is_ok());
    }

    #[test]
    fn resource_kinds_serialize_snake_case() {
        assert_eq!(
            serde_json::to_value(ResourceKind::PromptSection).expect("json"),
            serde_json::json!("prompt_section")
        );
        assert_eq!(
            serde_json::to_value(ResourceKind::Subagent).expect("json"),
            serde_json::json!("subagent")
        );
    }
}
