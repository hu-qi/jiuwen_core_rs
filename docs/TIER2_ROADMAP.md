# 第二梯队任务总纲(TIER-2 ROADMAP)

> 状态:规划者(主对话)制定任务书,分派给独立执行对话实现,由规划者验收。
> 验收标准:构建通过、全部测试通过、clippy 0 警告、实现真实(非 mock/占位)、
> 符合插件架构(契约在 ah-contracts,插件在 ah-plugins-*,注册返回 Effect,
> profile 与 ah-app catalog 接线,文档同步更新)。
>
> **当前状态(第 50 回合账目)**:273 tests / clippy 0 / fmt clean。
> 本路线图任务 4-6 已全部验收;其后第三梯队 A(teams/evolving/rsi)、B(工程收尾)
> 与 C(域深化)各项也已落地,逐回合账目见 [REMAINING_PLAN.md](REMAINING_PLAN.md)。

## 任务 4:MCP stdio transport ✅ 已验收

- 契约 `ah-contracts/src/mcp.rs`:`McpClient` seam(initialize/list_tools/call_tool/shutdown)、
  `McpTool`/`McpToolResult`/`McpInfo`/`McpError` 类型、`MCP` 服务键;
- 实现 `ah-plugins-mcp`:`StdioMcpClient` —— 真实子进程 + newline-delimited JSON-RPC 2.0,
  真实 initialize 握手;
- 测试:真实子进程(本地 fake MCP server 脚本)验证握手/list_tools/call_tool/错误映射;
- 接线:workspace/profiles/ah-app catalog。
- 后续补充:mcp-http 客户端已落地(0293962,streamable-HTTP 风格 POST JSON-RPC,本地 HTTP 端点往返验证)。

## 任务 5:telemetry / OTel 基础 ✅ 已验收

- 契约 `ah-contracts/src/telemetry.rs`:`TelemetryProvider` seam(record_span/spans/export)、
  `Span` 类型(name/parent/attributes/start_ms/duration_ms)、`TELEMETRY` 服务键;
- 实现 `ah-plugins-telemetry`:真实 span 记录 + JSONL 文件导出(持久化),
  通过事件监听(agent/step、tools/post-execute)生成 span;
- 测试:agent 运行产生真实 span、导出 JSONL 可重读;
- 接线:workspace/profiles/ah-app catalog。OTLP 导出原留作后续(文档注明),现已落地(53afa6a:resourceSpans/scopeSpans/spans 编码 + POST collector);semconv 留待后续。

## 任务 6:credentials seam ✅ 已验收

- 契约 `ah-contracts/src/credentials.rs`:`CredentialProvider` seam(get/set/list/remove)、
  `Credential` 类型、`CREDENTIALS` 服务键;
- 实现 `ah-plugins-credentials`:真实环境变量 provider(如 `OPENAI_API_KEY` → `openai.api_key`);
- 集成:让 `ah-plugins-openai` 的 OpenAiConfig 可从 credentials seam 读取 key(可选但优先);
- 测试:env provider 读写、openai 插件经 credentials 解析 key(集成测试覆盖真实 HTTP 往返);
- 接线:workspace/profiles/ah-app catalog。
---

## 完成状态(2026-08-14 验收时点)

| 任务 | 提交 | 验收 |
| --- | --- | --- |
| 4. MCP stdio transport | `13b7594`(+验收补丁 `b8bd840`) | ✅ 79 测试,真实子进程 JSON-RPC |
| 5. telemetry | `3ed4157` | ✅ 83 测试,JSONL span 导出真实 |
| 6. credentials | `e016198` | ✅ 101 测试,env provider + openai 集成 |

## 后续账目(截至第 50 回合,2026-08-16)

- 任务 4-6 交付后,验收时点全仓 101 测试;当前全仓 **273 tests / clippy 0 / fmt clean**,
  最新账目见 [REMAINING_PLAN.md](REMAINING_PLAN.md)(第 50 回合)。
- 本路线图已无"规划中"条目:第三梯队 A(teams/evolving/rsi)、B(契约 fixtures G-02、
  覆盖率门禁 ≥80%、mock 门禁 0f55d52)与 C 域深化(context/store/prompt/retrieval/
  rails/workspace/graph-memory/streaming/anthropic/runner/controller/operator/
  optimizer/trainer/external/queue-redis/store-pg/symphony/agentbuilder/oauth/
  OTLP 导出/ApprovalRail/tune/pregel/A2A SSE/skill/checkpoint 续跑/类型化子代理/
  RL reward/auto-harness/evolution analyzer/swarmflow 等)均已落地并验收。
