# 第二梯队任务总纲(TIER-2 ROADMAP)

> 状态:规划者(主对话)制定任务书,分派给独立执行对话实现,由规划者验收。
> 验收标准:构建通过、全部测试通过、clippy 0 警告、实现真实(非 mock/占位)、
> 符合插件架构(契约在 ah-contracts,插件在 ah-plugins-*,注册返回 Effect,
> profile 与 ah-app catalog 接线,文档同步更新)。

## 任务 4:MCP stdio transport

- 契约 `ah-contracts/src/mcp.rs`:`McpClient` seam(initialize/list_tools/call_tool/shutdown)、
  `McpTool`/`McpToolResult`/`McpInfo`/`McpError` 类型、`MCP` 服务键;
- 实现 `ah-plugins-mcp`:`StdioMcpClient` —— 真实子进程 + newline-delimited JSON-RPC 2.0,
  真实 initialize 握手;
- 测试:真实子进程(本地 fake MCP server 脚本)验证握手/list_tools/call_tool/错误映射;
- 接线:workspace/profiles/ah-app catalog。

## 任务 5:telemetry / OTel 基础

- 契约 `ah-contracts/src/telemetry.rs`:`TelemetryProvider` seam(record_span/spans/export)、
  `Span` 类型(name/parent/attributes/start_ms/duration_ms)、`TELEMETRY` 服务键;
- 实现 `ah-plugins-telemetry`:真实 span 记录 + JSONL 文件导出(持久化),
  通过事件监听(agent/step、tools/post-execute)生成 span;
- 测试:agent 运行产生真实 span、导出 JSONL 可重读;
- 接线:workspace/profiles/ah-app catalog。OTLP 导出留作后续(文档注明)。

## 任务 6:credentials seam

- 契约 `ah-contracts/src/credentials.rs`:`CredentialProvider` seam(get/set/list/remove)、
  `Credential` 类型、`CREDENTIALS` 服务键;
- 实现 `ah-plugins-credentials`:真实环境变量 provider(如 `OPENAI_API_KEY` → `openai.api_key`);
- 集成:让 `ah-plugins-openai` 的 OpenAiConfig 可从 credentials seam 读取 key(可选但优先);
- 测试:env provider 读写、openai 插件经 credentials 解析 key;
- 接线:workspace/profiles/ah-app catalog。
---

## 完成状态(2026-08-14)

| 任务 | 提交 | 验收 |
| --- | --- | --- |
| 4. MCP stdio transport | `13b7594`(+验收补丁 `b8bd840`) | ✅ 79 测试,真实子进程 JSON-RPC |
| 5. telemetry | `3ed4157` | ✅ 83 测试,JSONL span 导出真实 |
| 6. credentials | `e016198` | ✅ 101 测试,env provider + openai 集成 |

第二梯队全部完成:101 测试全过、clippy 0 警告。
