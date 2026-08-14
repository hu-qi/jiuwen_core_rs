//! MCP stdio transport 集成测试:真实子进程(fake MCP server 脚本)+ 真实协议往返。
//!
//! 测试 spawn 的是 tests/fake_mcp_server.sh —— 一个真实子进程,通过 stdin/stdout
//! 讲 newline-delimited JSON-RPC 2.0。客户端每行写一个 JSON 对象,按 id 匹配应答;
//! 握手 / list_tools / call_tool / 错误映射 / shutdown 全部走真实协议路径。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::keys::MCP;
use ah_contracts::mcp::{McpClient, McpContent};
use ah_contracts::prelude::Effect;
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_plugins_mcp::McpPlugin;
use ah_plugins_mcp::client::StdioMcpClient;
use serde_json::json;

/// fake MCP server 脚本的绝对路径。
fn fake_server() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake_mcp_server.sh").to_string()
}

/// 每次测试独立的退出标记目录(脚本退出时在其中创建 exited 文件)。
fn marker_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ah-mcp-marker-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create marker dir");
    dir
}

#[tokio::test]
async fn stdio_client_real_subprocess_roundtrip() {
    let marker = marker_dir("roundtrip");
    // 直接 spawn 真实子进程脚本(标记目录作为 $1 传入)。
    let client = StdioMcpClient::new(fake_server(), vec![marker.to_string_lossy().into_owned()]);

    // 真实 initialize 握手。
    let info = client.initialize().await.expect("initialize");
    assert_eq!(info.protocol_version, "2024-11-05");
    assert_eq!(info.server_name, "fake-mcp-server");

    // list_tools:解析 server 返回的工具与 inputSchema。
    let tools = client.list_tools().await.expect("list_tools");
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["echo", "add"]);
    assert_eq!(
        tools[0].input_schema["properties"]["text"]["type"],
        "string"
    );

    // call_tool:文本往返。
    let echoed = client
        .call_tool("echo", json!({ "text": "hello from client" }))
        .await
        .expect("call echo");
    assert!(!echoed.is_error);
    assert_eq!(
        echoed.content,
        vec![McpContent::Text("echo:hello from client".to_string())]
    );

    // call_tool:整数求和(server 侧真实计算)。
    let sum = client
        .call_tool("add", json!({ "a": 2, "b": 40 }))
        .await
        .expect("call add");
    assert_eq!(sum.content, vec![McpContent::Text("42".to_string())]);

    // 真实 shutdown:shutdown 请求 + exit 通知,子进程真实退出。
    client.shutdown().await.expect("shutdown");
    assert!(
        marker.join("exited").exists(),
        "fake server process must actually exit after shutdown"
    );
    let _ = std::fs::remove_dir_all(&marker);
}

#[tokio::test]
async fn unknown_tool_maps_to_jsonrpc_error() {
    let marker = marker_dir("error");
    let client = StdioMcpClient::new(fake_server(), vec![marker.to_string_lossy().into_owned()]);
    client.initialize().await.expect("initialize");

    // 未知工具 -> server 返回 JSON-RPC error -> 客户端映射为 McpError。
    let error = client
        .call_tool("no_such_tool", json!({}))
        .await
        .expect_err("unknown tool must error");
    assert!(error.0.contains("-32602"), "got: {}", error.0);
    assert!(
        error.0.contains("Unknown tool: no_such_tool"),
        "got: {}",
        error.0
    );

    client.shutdown().await.expect("shutdown");
    let _ = std::fs::remove_dir_all(&marker);
}

#[tokio::test]
async fn shutdown_is_idempotent_and_server_exits() {
    let marker = marker_dir("twice");
    let client = StdioMcpClient::new(fake_server(), vec![marker.to_string_lossy().into_owned()]);
    client.initialize().await.expect("initialize");

    client.shutdown().await.expect("first shutdown");
    client
        .shutdown()
        .await
        .expect("second shutdown (idempotent)");
    assert!(marker.join("exited").exists());
    let _ = std::fs::remove_dir_all(&marker);
}

#[tokio::test]
async fn missing_server_command_fails_explicitly() {
    let client = StdioMcpClient::new("definitely-not-a-real-mcp-server-xyz", Vec::new());
    // 不可执行命令 -> 显式错误(不静默 fallback)。
    let error = client.initialize().await.expect_err("spawn must fail");
    assert!(error.0.contains("spawn mcp server"), "got: {}", error.0);
    // 从未 spawn 的客户端 shutdown 是幂等 no-op。
    client.shutdown().await.expect("shutdown no-op");
}

#[tokio::test]
async fn plugin_registers_seam_and_mcp_call_tool() {
    let marker = marker_dir("plugin");
    let ctx = Context::new();
    let plugins: Vec<DynPlugin> = vec![
        Arc::new(ah_plugins_tools::ToolsPlugin),
        Arc::new(McpPlugin::new(
            fake_server(),
            vec![marker.to_string_lossy().into_owned()],
        )),
    ];
    let effects: Vec<Effect> = ctx.mount_all(plugins).expect("mount");

    // MCP seam 解析 + 真实握手/工具调用。
    let mcp: Arc<dyn McpClient> = ctx.service(&MCP).expect("mcp seam");
    let info = mcp.initialize().await.expect("initialize via seam");
    assert_eq!(info.server_name, "fake-mcp-server");
    assert_eq!(mcp.list_tools().await.expect("list_tools").len(), 2);

    // mcp_call_tool 注册进 tools seam 并可经注册表真实调用。
    let registry = ctx
        .service::<dyn ToolRegistry>(&ah_contracts::keys::TOOLS)
        .expect("tools seam");
    assert!(registry.names().contains(&"mcp_call_tool".to_string()));
    let output = registry
        .invoke(
            "mcp_call_tool",
            json!({ "tool": "echo", "arguments": { "text": "via tool" } }),
        )
        .await
        .expect("mcp_call_tool");
    assert_eq!(output["content"], "echo:via tool");
    assert_eq!(output["is_error"], false);

    mcp.shutdown().await.expect("shutdown");
    drop(effects);
    // 卸载后 mcp seam 反注册。
    assert!(!ctx.has_service(&MCP));
    assert!(marker.join("exited").exists());
    let _ = std::fs::remove_dir_all(&marker);
}
