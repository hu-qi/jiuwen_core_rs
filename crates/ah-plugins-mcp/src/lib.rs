//! # ah-plugins-mcp
//!
//! 真实 MCP stdio transport:通过 tokio 子进程的 stdin/stdout 讲
//! newline-delimited JSON-RPC 2.0(每行一个 JSON 对象;request 带 `id`,
//! response 按 `id` 匹配),真实 `initialize` 握手与 `shutdown`。
//!
//! 本插件没有 mock:子进程在第一次方法调用时真实 spawn
//! (插件 apply 只注册 seam,不拉起外部命令),所有调用都是真实协议往返。

pub mod client;
pub mod http;
pub use client::StdioMcpClient;

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::keys::{MCP, TOOLS};
use ah_contracts::mcp::{McpClient, McpContent};
use ah_contracts::prelude::Effect;
use ah_contracts::service::ServiceKey;
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

/// MCP 插件:注册 `mcp` seam,并把 `mcp_call_tool` 注入 tools seam。
///
/// 服务器命令与参数来自 `McpPlugin::new`(如默认 `npx` + MCP server 包);
/// 客户端懒 spawn,apply 本身不拉起外部进程。
pub struct McpPlugin {
    command: String,
    args: Vec<String>,
}

impl McpPlugin {
    /// 以服务器命令与参数构造插件。
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            command: command.into(),
            args,
        }
    }
}

impl Plugin for McpPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mcp"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MCP]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let client: Arc<dyn McpClient> =
            Arc::new(StdioMcpClient::new(self.command.clone(), self.args.clone()));
        let mut effects = vec![ctx.register(MCP, client.clone())];

        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".to_string(),
                })?;
        effects.push(registry.register(Arc::new(McpCallTool::new(client))));
        Ok(effects)
    }
}

/// `mcp_call_tool` 工具:经 MCP seam 调用远端工具。
pub struct McpCallTool {
    mcp: Arc<dyn McpClient>,
}

impl McpCallTool {
    pub fn new(mcp: Arc<dyn McpClient>) -> Self {
        Self { mcp }
    }
}

#[async_trait]
impl Tool for McpCallTool {
    fn name(&self) -> &'static str {
        "mcp_call_tool"
    }

    fn description(&self) -> &'static str {
        "call a tool exposed by the MCP server; arguments: {tool, arguments?}"
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tool": { "type": "string" },
                "arguments": { "type": "object" },
            },
            "required": ["tool"],
        })
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let tool = arguments
            .get("tool")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError("missing string field tool".to_string()))?;
        let tool_arguments = arguments.get("arguments").cloned().unwrap_or(json!({}));
        let result = self
            .mcp
            .call_tool(tool, tool_arguments)
            .await
            .map_err(|e| ToolError(format!("mcp call failed: {e}")))?;
        let mut output = json!({"is_error": result.is_error, "content": ""});
        let mut text = Vec::new();
        let mut images = Vec::new();
        for item in result.content {
            match item {
                McpContent::Text(value) => text.push(value),
                McpContent::Image { mime_type, data } => {
                    images.push(json!({"mime_type":mime_type,"data":data}));
                }
            }
        }
        output["content"] = json!(text.join("\n"));
        if !images.is_empty() {
            output["images"] = json!(images);
        }
        Ok(output)
    }
}

/// Browser tool proxy backed by a real Playwright MCP client.
pub struct BrowserMcpTool {
    name: &'static str,
    client: Arc<dyn McpClient>,
}

impl BrowserMcpTool {
    pub fn new(name: &'static str, client: Arc<dyn McpClient>) -> Self {
        Self { name, client }
    }

    fn evaluate_function(&self, arguments: &Value) -> Option<String> {
        let max = arguments
            .get("max_items")
            .or_else(|| arguments.get("max_cards"))
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .min(100);
        let query = arguments.get("query").and_then(Value::as_str).unwrap_or("");
        let query = serde_json::to_string(query).ok()?;
        let operation = if self.name == "browser_custom_action" {
            arguments
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("")
        } else {
            self.name
        };
        match operation {
            "browser_probe_interactives" => Some(format!(
                r#"async () => {{
                  const q = {query}.toLowerCase();
                  const visible = e => {{ const r=e.getBoundingClientRect(), s=getComputedStyle(e); return r.width>0 && r.height>0 && s.visibility!=='hidden' && s.display!=='none'; }};
                  const text = e => (e.innerText || e.value || e.getAttribute('aria-label') || '').trim();
                  return JSON.stringify([...document.querySelectorAll('a,button,input,textarea,select,[role]')]
                    .filter(e => visible(e) && (!q || text(e).toLowerCase().includes(q)))
                    .slice(0, {max}).map(e => {{ const r=e.getBoundingClientRect(); return {{role:e.getAttribute('role') || e.tagName.toLowerCase(), text:text(e), aria_label:e.getAttribute('aria-label'), testid:e.getAttribute('data-testid'), bbox:{{x:r.x,y:r.y,width:r.width,height:r.height}}, selector_hint:e.id ? '#'+e.id : null}}; }}));
                }}"#
            )),
            "browser_probe_cards" => Some(format!(
                r#"async () => {{
                  const q = {query}.toLowerCase();
                  const visible = e => {{ const r=e.getBoundingClientRect(), s=getComputedStyle(e); return r.width>0 && r.height>0 && s.visibility!=='hidden' && s.display!=='none'; }};
                  const text = e => (e.innerText || '').trim();
                  return JSON.stringify([...document.querySelectorAll('article,[role="article"],li,tr')]
                    .filter(e => visible(e) && (!q || text(e).toLowerCase().includes(q)))
                    .slice(0, {max}).map(e => {{ const r=e.getBoundingClientRect(); const link=e.querySelector('a[href]'); return {{text:text(e).slice(0,500), primary_link:link?.href || null, bbox:{{x:r.x,y:r.y,width:r.width,height:r.height}}}}; }}));
                }}"#
            )),
            "browser_get_element_coordinates" => self.custom_coordinates_function(arguments),
            "browser_drag_and_drop" => self.custom_drag_function(arguments),
            _ => None,
        }
    }

    fn custom_coordinates_function(&self, arguments: &Value) -> Option<String> {
        let params = arguments
            .get("params")
            .cloned()
            .unwrap_or_else(|| arguments.clone());
        let payload = serde_json::to_string(&params).ok()?;
        Some(format!(
            r#"async () => {{
              const p = {payload};
              const point = (selector, x, y) => {{
                if (Number.isFinite(x) && Number.isFinite(y)) return {{x:Math.trunc(x),y:Math.trunc(y)}};
                if (!selector) return null;
                const e = document.querySelector(String(selector)); if (!e) return null;
                const r = e.getBoundingClientRect(); return r.width && r.height ? {{x:r.x+r.width/2,y:r.y+r.height/2}} : null;
              }};
              const source = point(p.element_source, p.coord_source_x, p.coord_source_y);
              const target = point(p.element_target, p.coord_target_x, p.coord_target_y);
              return JSON.stringify({{ok:!!source && (!p.element_target && !Number.isFinite(p.coord_target_x) ? true : !!target), source, target, error:source ? null : 'source element or coordinates not found'}});
            }}"#
        ))
    }

    fn custom_drag_function(&self, arguments: &Value) -> Option<String> {
        let params = arguments
            .get("params")
            .cloned()
            .unwrap_or_else(|| arguments.clone());
        let payload = serde_json::to_string(&params).ok()?;
        Some(format!(
            r#"async () => {{
              const p = {payload};
              const point = (selector, x, y) => {{ if (Number.isFinite(x) && Number.isFinite(y)) return {{x:Math.trunc(x),y:Math.trunc(y)}}; const e=selector&&document.querySelector(String(selector)); if(!e)return null; const r=e.getBoundingClientRect(); return r.width&&r.height?{{x:r.x+r.width/2,y:r.y+r.height/2}}:null; }};
              const source=point(p.element_source,p.coord_source_x,p.coord_source_y), target=point(p.element_target,p.coord_target_x,p.coord_target_y);
              if (!source || !target) return JSON.stringify({{ok:false,error:'source or target not found',source,target}});
              const emit=(type,pt)=>document.elementFromPoint(pt.x,pt.y)?.dispatchEvent(new PointerEvent(type,{{bubbles:true,clientX:pt.x,clientY:pt.y,pointerId:1}}));
              emit('pointerdown',source); emit('mousedown',source); emit('pointermove',target); emit('mousemove',target); emit('pointerup',target); emit('mouseup',target); emit('drop',target);
              return JSON.stringify({{ok:true,source,target,error:null}});
            }}"#
        ))
    }
}

#[async_trait]
impl Tool for BrowserMcpTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "Playwright browser operation through MCP"
    }
    fn parameters(&self) -> Value {
        json!({"type":"object"})
    }
    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        if self.name == "browser_list_custom_actions" {
            return Ok(json!({
                "ok": true,
                "actions": [
                    {"name":"browser_get_element_coordinates","params":["element_source","element_target","coord_source_x","coord_source_y","coord_target_x","coord_target_y"]},
                    {"name":"browser_drag_and_drop","params":["element_source","element_target","coord_source_x","coord_source_y","coord_target_x","coord_target_y","steps","delay_ms"]},
                    {"name":"list_upload_files","params":["root"]}
                ]
            }));
        }
        let (name, args) = if self.name == "browser_runtime_health" {
            ("browser_tabs", json!({}))
        } else if let Some(function) = self.evaluate_function(&arguments) {
            ("browser_evaluate", json!({"function": function}))
        } else if self.name == "browser_custom_action" {
            return Err(ToolError("unsupported browser custom action".into()));
        } else {
            (self.name, arguments)
        };
        let result = self
            .client
            .call_tool(name, args)
            .await
            .map_err(|error| ToolError(format!("browser MCP call failed: {error}")))?;
        let mut text = Vec::new();
        let mut images = Vec::new();
        for item in result.content {
            match item {
                McpContent::Text(value) => text.push(value),
                McpContent::Image { mime_type, data } => {
                    images.push(json!({"mime_type":mime_type,"data":data}))
                }
            }
        }
        if !result.is_error
            && images.is_empty()
            && text.len() == 1
            && let Ok(value) = serde_json::from_str::<Value>(&text[0])
            && (value.is_object() || value.is_array())
        {
            if self.name == "browser_runtime_health" {
                return Ok(
                    json!({"ok":true,"started":true,"last_heartbeat_ok":true,"browser_tabs":value}),
                );
            }
            return Ok(value);
        }

        let mut output = json!({"is_error": result.is_error, "content": text.join("\n")});
        if images.len() == 1 {
            let image = images.pop().expect("one image");
            output["mime_type"] = image["mime_type"].clone();
            output["data"] = image["data"].clone();
        } else if !images.is_empty() {
            output["images"] = json!(images);
        }
        Ok(output)
    }
}

const DEFAULT_PLAYWRIGHT_CAPABILITIES: &[&str] = &[
    "pdf", "vision", "devtools", "config", "network", "storage", "testing",
];

fn ensure_playwright_capabilities(mut args: Vec<String>) -> Vec<String> {
    if args
        .iter()
        .any(|arg| arg == "--caps" || arg.starts_with("--caps="))
    {
        return args;
    }
    args.push(format!(
        "--caps={}",
        DEFAULT_PLAYWRIGHT_CAPABILITIES.join(",")
    ));
    args
}

/// Registers the official Playwright MCP tools lazily.
pub struct BrowserMcpPlugin {
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
}

impl BrowserMcpPlugin {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Self {
        Self::new_with_options(command, args, None, HashMap::new())
    }

    pub fn new_with_options(
        command: impl Into<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            cwd,
            env,
        }
    }

    pub fn from_env() -> Self {
        let command = env::var("PLAYWRIGHT_MCP_COMMAND").unwrap_or_else(|_| "npx".into());
        let args = env::var("PLAYWRIGHT_MCP_ARGS")
            .ok()
            .map(|raw| raw.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(|| vec!["-y".into(), "@playwright/mcp".into(), "--headless".into()]);
        let args = ensure_playwright_capabilities(args);
        let cwd = ["PLAYWRIGHT_RUNTIME_MCP_CWD", "BROWSER_RUNTIME_MCP_CWD"]
            .into_iter()
            .find_map(|name| {
                env::var_os(name)
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
            });
        let mut child_env = HashMap::new();
        for name in [
            "PLAYWRIGHT_BROWSERS_PATH",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
        ] {
            if let Ok(value) = env::var(name)
                && !value.is_empty()
            {
                child_env.insert(name.to_string(), value);
            }
        }
        Self::new_with_options(command, args, cwd, child_env)
    }
}

impl Plugin for BrowserMcpPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-browser-mcp"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        Vec::new()
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![TOOLS]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let client: Arc<dyn McpClient> = Arc::new(StdioMcpClient::new_with_options(
            self.command.clone(),
            self.args.clone(),
            self.cwd.clone(),
            self.env.clone(),
        ));
        let registry =
            ctx.service::<dyn ToolRegistry>(&TOOLS)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "tools seam not registered".into(),
                })?;
        Ok([
            "browser_click",
            "browser_close",
            "browser_console_messages",
            "browser_drag",
            "browser_drop",
            "browser_evaluate",
            "browser_file_upload",
            "browser_fill_form",
            "browser_find",
            "browser_handle_dialog",
            "browser_hover",
            "browser_navigate",
            "browser_navigate_back",
            "browser_network_request",
            "browser_network_requests",
            "browser_press_key",
            "browser_resize",
            "browser_custom_action",
            "browser_list_custom_actions",
            "browser_run_code_unsafe",
            "browser_select_option",
            "browser_snapshot",
            "browser_tabs",
            "browser_take_screenshot",
            "browser_type",
            "browser_wait_for",
            "browser_pdf_save",
            "browser_mouse_click_xy",
            "browser_mouse_down",
            "browser_mouse_drag_xy",
            "browser_mouse_move_xy",
            "browser_mouse_up",
            "browser_mouse_wheel",
            "browser_annotate",
            "browser_hide_highlight",
            "browser_highlight",
            "browser_resume",
            "browser_start_tracing",
            "browser_start_video",
            "browser_stop_tracing",
            "browser_stop_video",
            "browser_video_chapter",
            "browser_video_hide_actions",
            "browser_video_show_actions",
            "browser_get_config",
            "browser_network_state_set",
            "browser_route",
            "browser_route_list",
            "browser_unroute",
            "browser_cookie_clear",
            "browser_cookie_delete",
            "browser_cookie_get",
            "browser_cookie_list",
            "browser_cookie_set",
            "browser_localstorage_clear",
            "browser_localstorage_delete",
            "browser_localstorage_get",
            "browser_localstorage_list",
            "browser_localstorage_set",
            "browser_sessionstorage_clear",
            "browser_sessionstorage_delete",
            "browser_sessionstorage_get",
            "browser_sessionstorage_list",
            "browser_sessionstorage_set",
            "browser_set_storage_state",
            "browser_storage_state",
            "browser_generate_locator",
            "browser_verify_element_visible",
            "browser_verify_list_visible",
            "browser_verify_text_visible",
            "browser_verify_value",
            "browser_probe_interactives",
            "browser_probe_cards",
            "browser_runtime_health",
        ]
        .into_iter()
        .map(|name| registry.register(Arc::new(BrowserMcpTool::new(name, client.clone()))))
        .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::mcp::{McpError, McpInfo, McpTool, McpToolResult};
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn default_capabilities_are_added_without_overriding_explicit_caps() {
        let defaulted = ensure_playwright_capabilities(vec!["--headless".into()]);
        assert!(defaulted.iter().any(|arg| arg == "--headless"));
        assert!(defaulted.iter().any(|arg| arg.starts_with("--caps=")));

        let explicit = ensure_playwright_capabilities(vec!["--caps=core".into()]);
        assert_eq!(explicit, vec!["--caps=core"]);
    }

    struct RecordingMcp {
        calls: Mutex<Vec<(String, Value)>>,
    }
    impl ah_contracts::seam::Seam for RecordingMcp {}
    #[async_trait]
    impl McpClient for RecordingMcp {
        async fn initialize(&self) -> Result<McpInfo, McpError> {
            unreachable!()
        }
        async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
            unreachable!()
        }
        async fn call_tool_with_timeout(
            &self,
            name: &str,
            arguments: Value,
            _timeout: std::time::Duration,
        ) -> Result<McpToolResult, McpError> {
            self.calls
                .lock()
                .map_err(|_| McpError("recording MCP lock poisoned".into()))?
                .push((name.to_string(), arguments));
            Ok(McpToolResult {
                content: vec![McpContent::Text("{\"ok\":true,\"elements\":[]}".into())],
                is_error: false,
            })
        }
        async fn shutdown(&self) -> Result<(), McpError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn interactive_probe_uses_browser_evaluate_and_returns_json() {
        let client = Arc::new(RecordingMcp {
            calls: Mutex::new(Vec::new()),
        });
        let tool = BrowserMcpTool::new("browser_probe_interactives", client.clone());
        let output = tool
            .invoke(json!({"max_items": 10, "query": "search"}))
            .await
            .unwrap();
        assert_eq!(output["ok"], true);
        let calls = client
            .calls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "browser_evaluate");
        assert!(calls[0].1["function"].as_str().unwrap().contains("search"));
    }

    #[tokio::test]
    async fn custom_drag_action_uses_browser_evaluate() {
        let client = Arc::new(RecordingMcp {
            calls: Mutex::new(Vec::new()),
        });
        let tool = BrowserMcpTool::new("browser_custom_action", client.clone());
        let output = tool
            .invoke(json!({"action":"browser_drag_and_drop","params":{"coord_source_x":1,"coord_source_y":2,"coord_target_x":3,"coord_target_y":4}}))
            .await
            .unwrap();
        assert_eq!(output["ok"], true);
        let calls = client
            .calls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(calls[0].0, "browser_evaluate");
        assert!(calls[0].1["function"].as_str().unwrap().contains("mouse"));
    }

    #[tokio::test]
    async fn runtime_health_queries_browser_tabs() {
        let client = Arc::new(RecordingMcp {
            calls: Mutex::new(Vec::new()),
        });
        let tool = BrowserMcpTool::new("browser_runtime_health", client.clone());
        let output = tool.invoke(json!({})).await.unwrap();
        assert_eq!(output["ok"], true);
        let calls = client
            .calls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(calls[0].0, "browser_tabs");
    }
}
