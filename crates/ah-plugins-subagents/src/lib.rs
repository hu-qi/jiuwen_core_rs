//! # ah-plugins-subagents
//!
//! Typed subagent builders matching the Python harness factories. Browser and
//! mobile builders keep their factory identity, prompts, tool allowlists and
//! task-scoped settings in the typed seam; execution remains isolated in the
//! shared `SubagentRuntime`.

use std::env;
use std::sync::Arc;
use std::time::Duration;

use ah_contracts::keys::{INTERRUPT, SESSION_MANAGER, SUBAGENT, SUBAGENTS};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::session::SessionManager;
use ah_contracts::skill_creator::encode_b64;
use ah_contracts::subagent::{SubagentResult, SubagentRuntime, SubagentSpec};
use ah_contracts::subagents::{
    SubagentKind, SubagentProfile, SubagentRequest, TypedSubagentError, TypedSubagents,
};
use ah_contracts::tools::{Tool, ToolError, ToolRegistry};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use serde_json::{Value, json};

const BROWSER_TOOLS: &[&str] = &[
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
    "browser_run_code_unsafe",
    "browser_select_option",
    "browser_snapshot",
    "browser_tabs",
    "browser_take_screenshot",
    "browser_type",
    "browser_wait_for",
    "browser_probe_interactives",
    "browser_probe_cards",
    "browser_batch_interact",
    "browser_custom_action",
    "browser_list_custom_actions",
    "browser_cancel_run",
    "browser_clear_cancel",
    "browser_runtime_health",
    "browser_pdf_save",
    "browser_mouse_click_xy",
    "browser_mouse_down",
    "browser_mouse_drag_xy",
    "browser_mouse_move_xy",
    "browser_mouse_up",
    "browser_mouse_wheel",
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
];

const MOBILE_TOOLS: &[&str] = &[
    "tap_coordinate",
    "double_tap_coordinate",
    "long_press_coordinate",
    "drag_coordinate",
    "type_text",
    "scroll",
    "press_back",
    "press_home",
    "press_enter",
    "wait_gui_load",
    "screenshot",
    "device_health",
];

/// Python browser factory 注入、但不属于 capability 分类的高层 helper。
const BROWSER_RUNTIME_HELPER_TOOLS: &[&str] = &[
    "browser_probe_interactives",
    "browser_probe_cards",
    "browser_batch_interact",
    "browser_custom_action",
    "browser_list_custom_actions",
];

fn profile_for(kind: SubagentKind) -> SubagentProfile {
    let (system_prompt, allowed_tools, default_budget) = match kind {
        SubagentKind::Code => (
            "You are a coding subagent. Inspect the workspace, make precise changes, and report exactly what you changed.",
            vec!["list_dir", "read_file", "write_file", "run_code"],
            20,
        ),
        SubagentKind::Research => (
            "You are a research subagent. Gather evidence from the workspace and the knowledge base; cite sources in your answer.",
            vec!["list_dir", "read_file", "search_knowledge", "web_fetch"],
            20,
        ),
        SubagentKind::Plan => (
            "You are a planning subagent. Produce a step-by-step plan with concrete, verifiable milestones; do not execute changes.",
            vec!["list_dir", "read_file"],
            12,
        ),
        SubagentKind::Verify => (
            "You are a verification subagent. Check the workspace against the task requirements and report pass/fail with evidence.",
            vec!["list_dir", "read_file", "run_code"],
            20,
        ),
        SubagentKind::Browser => (
            "You are a browser automation agent. Preserve browser session continuity, inspect before acting, and only claim completion when the requested outcome is evidenced. Prefer browser_probe_cards for repeated cards/listings and browser_probe_interactives for page controls before broad snapshots. Use browser_batch_interact only for already-grounded deterministic steps; use browser_custom_action only after discovering its contract.",
            BROWSER_TOOLS.to_vec(),
            25,
        ),
        SubagentKind::MobileGui => (
            "You are an Android GUI automation agent. Ground every coordinate action in the latest screenshot, use device_health before risky actions, preserve the requested device_serial, wait for UI settling after mutations, and verify the final state with a fresh screenshot before claiming completion. Return an explicit blocker when no device is reachable.",
            MOBILE_TOOLS.to_vec(),
            30,
        ),
    };
    SubagentProfile {
        kind,
        system_prompt: system_prompt.to_string(),
        allowed_tools: allowed_tools.into_iter().map(str::to_string).collect(),
        default_budget,
    }
}

fn inherited_context(
    manager: &dyn SessionManager,
    parent: Option<&str>,
) -> Result<Option<String>, TypedSubagentError> {
    let Some(parent) = parent.filter(|id| !id.trim().is_empty()) else {
        return Ok(None);
    };
    let session = manager
        .open(parent)
        .map_err(|e| TypedSubagentError(format!("open parent session failed: {e}")))?;
    let messages = session.derive_messages();
    if messages.is_empty() {
        return Ok(None);
    }
    let context = messages
        .into_iter()
        .map(|message| {
            let role = format!("{:?}", message.role).to_ascii_lowercase();
            format!("[{role}] {}", message.content)
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Some(format!(
        "Parent session context (read-only):\n{context}"
    )))
}
/// Resolve the task-scoped browser tool allowlist using the trusted catalog.
pub fn browser_tools_for_capabilities(
    capabilities: &[String],
) -> Result<Vec<String>, TypedSubagentError> {
    validate_browser_capabilities(capabilities)?;
    Ok(scoped_browser_tools(
        capabilities,
        BROWSER_TOOLS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    ))
}

fn scoped_browser_tools(capabilities: &[String], tools: Vec<String>) -> Vec<String> {
    let mut selected: Vec<&str> = BROWSER_TOOLS[..24].to_vec();
    for capability in capabilities {
        let names: &[&str] = match capability.as_str() {
            "core" => &[],
            "pdf" => &["browser_pdf_save"],
            "vision" => &[
                "browser_mouse_click_xy",
                "browser_mouse_down",
                "browser_mouse_drag_xy",
                "browser_mouse_move_xy",
                "browser_mouse_up",
                "browser_mouse_wheel",
            ],
            "devtools" => &[
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
            ],
            "config" => &["browser_get_config"],
            "network" => &[
                "browser_network_state_set",
                "browser_route",
                "browser_route_list",
                "browser_unroute",
            ],
            "storage" => &[
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
            ],
            "testing" => &[
                "browser_generate_locator",
                "browser_verify_element_visible",
                "browser_verify_list_visible",
                "browser_verify_text_visible",
                "browser_verify_value",
            ],
            _ => &[],
        };
        selected.extend(names);
    }
    tools
        .into_iter()
        .filter(|tool| selected.contains(&tool.as_str()))
        .collect()
}

fn validate_browser_capabilities(capabilities: &[String]) -> Result<(), TypedSubagentError> {
    const NAMES: &[&str] = &[
        "core", "pdf", "vision", "devtools", "config", "network", "storage", "testing",
    ];
    if let Some(name) = capabilities
        .iter()
        .find(|name| !NAMES.contains(&name.as_str()))
    {
        return Err(TypedSubagentError(format!(
            "unsupported browser capability: {name}"
        )));
    }
    Ok(())
}

/// Typed subagent runtime with concrete browser/mobile factory semantics.
pub struct TypedSubagentsImpl {
    subagent: Arc<dyn SubagentRuntime>,
    manager: Arc<dyn SessionManager>,
}

impl TypedSubagentsImpl {
    pub fn new(subagent: Arc<dyn SubagentRuntime>, manager: Arc<dyn SessionManager>) -> Self {
        Self { subagent, manager }
    }
}

impl Seam for TypedSubagentsImpl {}

#[async_trait]
impl TypedSubagents for TypedSubagentsImpl {
    fn kinds(&self) -> Vec<SubagentKind> {
        vec![
            SubagentKind::Code,
            SubagentKind::Research,
            SubagentKind::Plan,
            SubagentKind::Verify,
            SubagentKind::Browser,
            SubagentKind::MobileGui,
        ]
    }

    fn profile(&self, kind: SubagentKind) -> Option<SubagentProfile> {
        Some(profile_for(kind))
    }

    async fn run(
        &self,
        kind: SubagentKind,
        task: &str,
        budget: Option<usize>,
    ) -> Result<SubagentResult, TypedSubagentError> {
        self.run_request(SubagentRequest {
            kind,
            task: task.to_string(),
            budget,
            parent_session_id: None,
            browser_capabilities: Vec::new(),
            device_serial: None,
        })
        .await
    }

    async fn run_request(
        &self,
        request: SubagentRequest,
    ) -> Result<SubagentResult, TypedSubagentError> {
        if request.task.trim().is_empty() {
            return Err(TypedSubagentError("subagent task must not be empty".into()));
        }
        if request.kind != SubagentKind::Browser && !request.browser_capabilities.is_empty() {
            return Err(TypedSubagentError(
                "browser_capabilities are only valid for browser_agent".into(),
            ));
        }
        let profile = profile_for(request.kind);
        let allowed_tools = if request.kind == SubagentKind::Browser {
            let mut tools = browser_tools_for_capabilities(&request.browser_capabilities)?;
            tools.extend(
                BROWSER_RUNTIME_HELPER_TOOLS
                    .iter()
                    .map(|name| (*name).to_string()),
            );
            tools.sort();
            tools.dedup();
            tools
        } else {
            profile.allowed_tools.clone()
        };
        let inherited =
            inherited_context(self.manager.as_ref(), request.parent_session_id.as_deref())?;
        let settings = match request.kind {
            SubagentKind::Browser if !request.browser_capabilities.is_empty() => format!(
                "\nTask browser capabilities: {}",
                request.browser_capabilities.join(",")
            ),
            SubagentKind::MobileGui => format!(
                "\nAndroid device serial: {}. Include this exact value as device_serial in every Android GUI tool call.",
                request.device_serial.as_deref().unwrap_or("emulator-5554")
            ),
            _ => String::new(),
        };
        let context = [Some(profile.system_prompt), inherited, Some(settings)]
            .into_iter()
            .flatten()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        self.subagent
            .run(SubagentSpec {
                id: format!(
                    "{}-{}",
                    request.kind.factory_name(),
                    stable_id(&request.task)
                ),
                task: request.task,
                context: Some(context),
                budget: Some(request.budget.unwrap_or(profile.default_budget)),
                allowed_tools: Some(allowed_tools),
            })
            .await
            .map_err(|e| TypedSubagentError(e.0))
    }
}

fn stable_id(task: &str) -> String {
    let mut hash = 2166136261u32;
    for byte in task.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
    }
    format!("{hash:08x}")
}

/// subagents 插件:注入 SubagentRuntime,提供类型化子代理。
pub struct SubagentsPlugin;

impl Plugin for SubagentsPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-subagents"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![SUBAGENTS]
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![SUBAGENT, SESSION_MANAGER, INTERRUPT]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&SUBAGENT)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "subagent seam not registered".to_string(),
            })?;
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "session-manager seam not registered".to_string(),
            })?;
        let typed: Arc<dyn TypedSubagents> = Arc::new(TypedSubagentsImpl::new(subagent, manager));
        Ok(vec![ctx.register(SUBAGENTS, typed)])
    }
}

struct AdbTool {
    name: &'static str,
    command: String,
    serial: String,
}

impl AdbTool {
    fn new(name: &'static str, command: String, serial: String) -> Self {
        Self {
            name,
            command,
            serial,
        }
    }

    fn selected_serial(&self, arguments: &Value) -> String {
        arguments
            .get("device_serial")
            .and_then(Value::as_str)
            .filter(|serial| !serial.trim().is_empty())
            .unwrap_or(&self.serial)
            .to_string()
    }

    async fn run(&self, serial: &str, args: Vec<String>) -> Result<Vec<u8>, ToolError> {
        let output = tokio::process::Command::new(&self.command)
            .args(["-s", serial])
            .args(args)
            .output()
            .await
            .map_err(|error| ToolError(format!("adb spawn failed: {error}")))?;
        if !output.status.success() {
            return Err(ToolError(format!(
                "adb command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(output.stdout)
    }

    fn number(arguments: &Value, name: &str) -> Result<String, ToolError> {
        arguments
            .get(name)
            .and_then(Value::as_i64)
            .map(|value| value.to_string())
            .ok_or_else(|| ToolError(format!("missing integer field {name}")))
    }

    fn size(output: &[u8]) -> Option<(i64, i64)> {
        String::from_utf8_lossy(output)
            .split_whitespace()
            .find_map(|token| {
                let (width, height) = token.split_once('x')?;
                Some((width.parse().ok()?, height.parse().ok()?))
            })
    }

    fn foreground_app(output: &[u8]) -> Option<String> {
        String::from_utf8_lossy(output)
            .split_whitespace()
            .map(|token| token.trim_matches(|ch: char| "{}()".contains(ch)))
            .find(|token| token.contains('/') && !token.starts_with("Window"))
            .and_then(|token| token.split('/').next())
            .filter(|package| !package.is_empty())
            .map(str::to_string)
    }

    fn parameters_for(name: &str) -> Value {
        let mut properties = serde_json::Map::new();
        properties.insert("device_serial".into(), json!({"type":"string"}));
        match name {
            "tap_coordinate" | "double_tap_coordinate" | "long_press_coordinate" => {
                properties.insert("x".into(), json!({"type":"integer"}));
                properties.insert("y".into(), json!({"type":"integer"}));
            }
            "drag_coordinate" => {
                for field in ["start_x", "start_y", "end_x", "end_y"] {
                    properties.insert(field.into(), json!({"type":"integer"}));
                }
            }
            "type_text" => {
                properties.insert("text".into(), json!({"type":"string"}));
            }
            "scroll" => {
                properties.insert(
                    "direction".into(),
                    json!({"type":"string","enum":["up","down","left","right"]}),
                );
                properties.insert("duration_ms".into(), json!({"type":"integer","minimum":0}));
            }
            "wait_gui_load" => {
                properties.insert(
                    "seconds".into(),
                    json!({"type":"number","minimum":0,"maximum":30}),
                );
            }
            _ => {}
        }
        json!({"type":"object","properties":properties})
    }
}

#[async_trait]
impl Tool for AdbTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        "Android GUI operation through adb"
    }
    fn parameters(&self) -> Value {
        Self::parameters_for(self.name)
    }

    async fn invoke(&self, arguments: Value) -> Result<Value, ToolError> {
        let serial = self.selected_serial(&arguments);
        let result = match self.name {
            "tap_coordinate" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "tap".into(),
                        Self::number(&arguments, "x")?,
                        Self::number(&arguments, "y")?,
                    ],
                )
                .await?
            }
            "double_tap_coordinate" => {
                let x = Self::number(&arguments, "x")?;
                let y = Self::number(&arguments, "y")?;
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "tap".into(),
                        x.clone(),
                        y.clone(),
                    ],
                )
                .await?;
                self.run(
                    &serial,
                    vec!["shell".into(), "input".into(), "tap".into(), x, y],
                )
                .await?
            }
            "long_press_coordinate" => {
                let x = Self::number(&arguments, "x")?;
                let y = Self::number(&arguments, "y")?;
                let duration = arguments
                    .get("duration")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0)
                    .max(0.0)
                    * 1000.0;
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "swipe".into(),
                        x.clone(),
                        y.clone(),
                        x,
                        y,
                        format!("{duration:.0}"),
                    ],
                )
                .await?
            }
            "drag_coordinate" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "swipe".into(),
                        Self::number(&arguments, "start_x")?,
                        Self::number(&arguments, "start_y")?,
                        Self::number(&arguments, "end_x")?,
                        Self::number(&arguments, "end_y")?,
                        arguments
                            .get("duration")
                            .and_then(Value::as_u64)
                            .unwrap_or(300)
                            .to_string(),
                    ],
                )
                .await?
            }
            "type_text" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "text".into(),
                        arguments
                            .get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| ToolError("missing string field text".into()))?
                            .replace(' ', "%s"),
                    ],
                )
                .await?
            }
            "press_back" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "keyevent".into(),
                        "4".into(),
                    ],
                )
                .await?
            }
            "press_home" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "keyevent".into(),
                        "3".into(),
                    ],
                )
                .await?
            }
            "press_enter" => {
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "keyevent".into(),
                        "66".into(),
                    ],
                )
                .await?
            }
            "scroll" => {
                let direction = arguments
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("down");
                let (width, height) = self
                    .run(&serial, vec!["shell".into(), "wm".into(), "size".into()])
                    .await
                    .ok()
                    .and_then(|output| Self::size(&output))
                    .unwrap_or((1080, 1920));
                let (sx, sy, ex, ey) = match direction {
                    "up" => (width / 2, height * 2 / 10, width / 2, height * 8 / 10),
                    "left" => (width * 8 / 10, height / 2, width * 2 / 10, height / 2),
                    "right" => (width * 2 / 10, height / 2, width * 8 / 10, height / 2),
                    "down" => (width / 2, height * 8 / 10, width / 2, height * 2 / 10),
                    other => return Err(ToolError(format!("invalid scroll direction: {other}"))),
                };
                self.run(
                    &serial,
                    vec![
                        "shell".into(),
                        "input".into(),
                        "swipe".into(),
                        sx.to_string(),
                        sy.to_string(),
                        ex.to_string(),
                        ey.to_string(),
                        arguments
                            .get("duration_ms")
                            .and_then(Value::as_u64)
                            .unwrap_or(300)
                            .to_string(),
                    ],
                )
                .await?
            }
            "wait_gui_load" => {
                let seconds = arguments
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .unwrap_or(2.0)
                    .clamp(0.0, 30.0);
                tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
                Vec::new()
            }
            "screenshot" => {
                self.run(
                    &serial,
                    vec!["exec-out".into(), "screencap".into(), "-p".into()],
                )
                .await?
            }
            "device_health" => {
                self.run(
                    &serial,
                    vec!["shell".into(), "getprop".into(), "ro.product.model".into()],
                )
                .await?
            }
            _ => return Err(ToolError(format!("unsupported adb tool {}", self.name))),
        };
        if self.name == "screenshot" {
            let foreground_app = self
                .run(
                    &serial,
                    vec![
                        "shell".into(),
                        "dumpsys".into(),
                        "window".into(),
                        "windows".into(),
                    ],
                )
                .await
                .ok()
                .and_then(|output| Self::foreground_app(&output));
            Ok(json!({
                "mime_type":"image/png",
                "data":encode_b64(&result,"image/png"),
                "device_serial":serial,
                "foreground_app":foreground_app
            }))
        } else if self.name == "device_health" {
            Ok(
                json!({"ok":true,"device_serial":serial,"model":String::from_utf8_lossy(&result).trim()}),
            )
        } else {
            Ok(json!({"ok":true,"device_serial":serial,"tool":self.name}))
        }
    }
}
/// Registers Android GUI tools backed by the adb executable.
pub struct MobileAdbPlugin {
    command: String,
    serial: String,
}

impl MobileAdbPlugin {
    pub fn new(command: impl Into<String>, serial: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            serial: serial.into(),
        }
    }
    pub fn from_env() -> Self {
        Self::new(
            env::var("ANDROID_ADB_COMMAND").unwrap_or_else(|_| "adb".into()),
            env::var("DEVICE_SERIAL").unwrap_or_else(|_| "emulator-5554".into()),
        )
    }
}

impl Plugin for MobileAdbPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-mobile-adb"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        Vec::new()
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![ah_contracts::keys::TOOLS]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let registry = ctx
            .service::<dyn ToolRegistry>(&ah_contracts::keys::TOOLS)
            .ok_or_else(|| PluginError::Apply {
                plugin: self.name(),
                message: "tools seam not registered".into(),
            })?;
        Ok([
            "tap_coordinate",
            "double_tap_coordinate",
            "long_press_coordinate",
            "drag_coordinate",
            "type_text",
            "scroll",
            "press_back",
            "press_home",
            "press_enter",
            "wait_gui_load",
            "screenshot",
            "device_health",
        ]
        .into_iter()
        .map(|name| {
            registry.register(Arc::new(AdbTool::new(
                name,
                self.command.clone(),
                self.serial.clone(),
            )))
        })
        .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::{SESSION_MANAGER, SUBAGENTS};
    use ah_contracts::session::SessionManager;
    use ah_hub::plugin::DynPlugin;
    use std::sync::Arc as StdArc;

    fn build_ctx(root: &std::path::Path) -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        std::fs::create_dir_all(root).expect("test root");
        let adb = root.join("adb");
        std::fs::write(
            &adb,
            "#!/bin/sh\nif [ \"$2\" = exec-out ]; then printf '\\211PNG\\r\\n'; elif [ \"$3\" = dumpsys ]; then printf 'mCurrentFocus=Window{1 u0 test.app/.Main}'; fi\n",
        )
        .expect("fake adb");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&adb, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let session_dir = root.join("sessions");
        let default_path = session_dir.join("default.jsonl");
        let plugins: Vec<DynPlugin> = vec![
            StdArc::new(ah_plugins_mock::MockPlugin),
            StdArc::new(ah_plugins_tools::ToolsPlugin),
            StdArc::new(MobileAdbPlugin::new(
                adb.display().to_string(),
                "emulator-5554",
            )),
            StdArc::new(ah_plugins_sysop::SysopPlugin::new(root)),
            StdArc::new(ah_plugins_agent_control::AgentControlPlugin),
            StdArc::new(ah_plugins_session_log::SessionLogPlugin::new(
                &default_path,
                &session_dir,
            )),
            StdArc::new(ah_plugins_subagent::SubagentPlugin),
            StdArc::new(SubagentsPlugin),
        ];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[tokio::test]
    async fn typed_subagent_injects_kind_prompt_and_runs() {
        let root = std::env::temp_dir().join(format!("ah-ts-run-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");

        let profile = typed.profile(SubagentKind::Code).expect("profile");
        assert!(profile.system_prompt.contains("coding subagent"));
        assert!(profile.allowed_tools.contains(&"write_file".to_string()));

        let result = typed
            .run(SubagentKind::Code, "refactor the workspace", Some(6))
            .await
            .expect("run");
        assert!(result.answer.contains("mock final answer"));

        // 类型提示真实注入:子代理会话含 system 消息。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let session = manager
            .open(&format!(
                "code_agent-{}",
                stable_id("refactor the workspace")
            ))
            .expect("open");
        let messages = session.derive_messages();
        assert!(
            messages.iter().any(|m| {
                m.role == ah_contracts::llm::ChatRole::System
                    && m.content.contains("coding subagent")
            }),
            "kind system prompt injected into the subagent session"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn allowlist_blocks_disallowed_tool_calls() {
        let root = std::env::temp_dir().join(format!("ah-ts-allow-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");
        // Plan 白名单只含 list_dir/read_file;mock 模型会请求 list_dir → 放行。
        let result = typed
            .run(SubagentKind::Plan, "plan the next milestone", Some(4))
            .await
            .expect("run");
        assert!(result.answer.contains("mock final answer"));

        // 直接构造一个不允许任何工具的 spec:越权调用必须被拒(日志可见 "tool not allowed")。
        let subagent = ctx
            .service::<dyn SubagentRuntime>(&ah_contracts::keys::SUBAGENT)
            .expect("subagent");
        let res = subagent
            .run(SubagentSpec {
                id: format!("no-tools-{}", std::process::id()),
                task: "list the workspace".to_string(),
                context: None,
                budget: Some(3),
                allowed_tools: Some(vec![]),
            })
            .await
            .expect("run");
        assert!(res.answer.contains("mock final answer"));
        // mock 请求的 list_dir 未被白名单放行 → 会话日志含 "tool not allowed"。
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        let session_id = format!("no-tools-{}", std::process::id());
        let session = manager.open(&session_id).expect("open");
        let events = session.events();
        assert!(
            events.iter().any(|e| {
                e.payload
                    .get("output")
                    .and_then(serde_json::Value::as_str)
                    .map(|s| s.contains("tool not allowed"))
                    .unwrap_or(false)
            }),
            "disallowed tool call recorded as rejected"
        );

        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn browser_and_mobile_builders_inherit_context_and_validate_settings() {
        let root = std::env::temp_dir().join(format!("ah-ts-specialized-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");
        assert_eq!(
            typed.profile(SubagentKind::Browser).unwrap().default_budget,
            25
        );
        assert!(
            typed
                .profile(SubagentKind::Browser)
                .unwrap()
                .allowed_tools
                .contains(&"browser_navigate".into())
        );
        assert!(
            typed
                .profile(SubagentKind::MobileGui)
                .unwrap()
                .allowed_tools
                .contains(&"tap_coordinate".into())
        );
        let browser_tools = typed.profile(SubagentKind::Browser).unwrap().allowed_tools;
        let core_only = scoped_browser_tools(&["core".into()], browser_tools.clone());
        assert!(!core_only.contains(&"browser_pdf_save".to_string()));
        assert!(!core_only.contains(&"browser_probe_cards".to_string()));
        let pdf_tools = scoped_browser_tools(&["pdf".into()], browser_tools);
        assert!(pdf_tools.contains(&"browser_pdf_save".to_string()));
        let manager = ctx
            .service::<dyn SessionManager>(&SESSION_MANAGER)
            .expect("manager");
        manager
            .create("parent")
            .unwrap()
            .append(
                ah_contracts::session::SessionEventKind::User,
                serde_json::json!({"content": "keep this goal"}),
            )
            .unwrap();
        let result = typed
            .run_request(ah_contracts::subagents::SubagentRequest {
                kind: SubagentKind::MobileGui,
                task: "inspect the screen".into(),
                budget: Some(2),
                parent_session_id: Some("parent".into()),
                browser_capabilities: Vec::new(),
                device_serial: Some("emulator-5556".into()),
            })
            .await
            .expect("mobile run");
        assert!(result.answer.contains("mock final answer"));
        let child = manager
            .open(&format!(
                "mobile_gui_agent-{}",
                stable_id("inspect the screen")
            ))
            .unwrap();
        let system = child
            .derive_messages()
            .into_iter()
            .find(|message| message.role == ah_contracts::llm::ChatRole::System)
            .unwrap();
        assert!(system.content.contains("keep this goal"));
        let browser = typed
            .run_request(ah_contracts::subagents::SubagentRequest {
                kind: SubagentKind::Browser,
                task: "browse grounded cards".into(),
                budget: Some(2),
                parent_session_id: None,
                browser_capabilities: vec!["core".into()],
                device_serial: None,
            })
            .await
            .expect("browser run");
        assert!(browser.answer.contains("mock final answer"));
        let browser_session = manager
            .open(&format!(
                "browser_agent-{}",
                stable_id("browse grounded cards")
            ))
            .unwrap();
        let browser_system = browser_session
            .derive_messages()
            .into_iter()
            .find(|message| message.role == ah_contracts::llm::ChatRole::System)
            .unwrap();
        assert!(browser_system.content.contains("browser_probe_cards"));
        assert!(
            browser_system
                .content
                .contains("browser_probe_interactives")
        );

        assert!(
            typed
                .run_request(ah_contracts::subagents::SubagentRequest {
                    kind: SubagentKind::Browser,
                    task: "browse".into(),
                    budget: Some(1),
                    parent_session_id: None,
                    browser_capabilities: vec!["not-a-capability".into()],
                    device_serial: None,
                })
                .await
                .is_err()
        );
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn run_many_aggregates_successes_and_failures() {
        let root = std::env::temp_dir().join(format!("ah-ts-aggregate-{}", std::process::id()));
        let (ctx, effects) = build_ctx(&root);
        let typed = ctx
            .service::<dyn TypedSubagents>(&SUBAGENTS)
            .expect("subagents");
        let aggregate = typed
            .run_many(vec![
                ah_contracts::subagents::SubagentRequest {
                    kind: SubagentKind::Plan,
                    task: "plan".into(),
                    budget: Some(2),
                    parent_session_id: None,
                    browser_capabilities: Vec::new(),
                    device_serial: None,
                },
                ah_contracts::subagents::SubagentRequest {
                    kind: SubagentKind::Plan,
                    task: "".into(),
                    budget: Some(2),
                    parent_session_id: None,
                    browser_capabilities: Vec::new(),
                    device_serial: None,
                },
            ])
            .await;
        assert_eq!(
            (aggregate.total, aggregate.succeeded, aggregate.failed),
            (2, 1, 1)
        );
        assert_eq!(aggregate.answers.len(), 1);
        drop(effects);
        let _ = std::fs::remove_dir_all(&root);
    }
    #[tokio::test]
    async fn browser_mcp_proxy_roundtrips_through_stdio_server() {
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../ah-plugins-mcp/tests/fake_mcp_server.sh");
        let client: Arc<dyn ah_contracts::mcp::McpClient> = Arc::new(
            ah_plugins_mcp::StdioMcpClient::new("sh", vec![script.display().to_string()]),
        );
        let info = client.initialize().await.expect("initialize MCP");
        assert_eq!(info.server_name, "fake-mcp-server");
        let tool = ah_plugins_mcp::BrowserMcpTool::new("echo", client);
        let result = tool
            .invoke(json!({"text":"browser-provider"}))
            .await
            .expect("browser MCP call");
        assert_eq!(result["is_error"], false);
        assert_eq!(result["content"], "echo:browser-provider");
    }

    #[tokio::test]
    async fn mobile_adb_provider_executes_real_command_and_screenshot() {
        let root = std::env::temp_dir().join(format!("ah-adb-provider-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("args");
        let script = root.join("adb");
        std::fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$2\" = exec-out ]; then printf '\\\\211PNG\\\\r\\\\n'; fi\n", marker.display())).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tap = AdbTool::new(
            "tap_coordinate",
            script.display().to_string(),
            "device-1".into(),
        );
        let result = tap
            .invoke(json!({"x": 10, "y": 20}))
            .await
            .expect("adb tap");
        assert_eq!(result["ok"], true);
        assert!(
            std::fs::read_to_string(&marker)
                .unwrap()
                .contains("-s device-1 shell input tap 10 20")
        );
        let screenshot = AdbTool::new(
            "screenshot",
            script.display().to_string(),
            "device-1".into(),
        );
        let result = screenshot.invoke(json!({})).await.expect("adb screenshot");
        assert_eq!(result["mime_type"], "image/png");
        assert!(
            result["data"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn mobile_tools_support_key_events_and_request_device_override() {
        let root = std::env::temp_dir().join(format!("ah-adb-keys-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("args");
        let script = root.join("adb");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s' \"$*\" >> '{}'\nprintf '\\n' >> '{}'\n",
                marker.display(),
                marker.display()
            ),
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let home = AdbTool::new("press_home", script.display().to_string(), "default".into());
        let home_result = home
            .invoke(json!({"device_serial":"device-2"}))
            .await
            .expect("home");
        assert_eq!(home_result["ok"], true);
        let enter = AdbTool::new(
            "press_enter",
            script.display().to_string(),
            "default".into(),
        );
        enter.invoke(json!({})).await.expect("enter");
        let args = std::fs::read_to_string(&marker).unwrap();
        assert!(args.contains("-s device-2 shell input keyevent 3"));
        assert!(args.contains("-s default shell input keyevent 66"));
        assert_eq!(
            home.parameters()["properties"]["device_serial"]["type"],
            "string"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_foreground_android_package_from_dumpsys() {
        let output = b"mCurrentFocus=Window{123 u0 com.example.app/.MainActivity}";
        assert_eq!(
            AdbTool::foreground_app(output).as_deref(),
            Some("com.example.app")
        );
    }
}
