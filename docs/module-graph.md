# 模块依赖图(module-graph.md)

> 等价 DSH 的 module-graph:登记 crate 布局与依赖规则。新增 crate 时必须更新本文件。

## 1. 当前 crate 依赖

```text
ah-app ──> ah-hub ──> ah-contracts
   │          │
   ├─> ah-plugins-mock   (仅 llm boot 桩)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-tools  (真实工具注册表)    ──> ah-hub, ah-contracts
   ├─> ah-plugins-sysop  (真实 fs/shell)    ──> ah-hub, ah-contracts
   └─> ah-plugins-openai (真实 LLM HTTP)    ──> ah-hub, ah-contracts
   ├─> ah-plugins-agent-loop (真实 ReAct)  ──> ah-hub, ah-contracts
   ├─> ah-plugins-rails (ShellGuard/PathGuard/ToolBudget rails) ──> ah-hub, ah-contracts
   ├─> ah-plugins-session-log (JSONL 日志)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-workflow (工作流引擎)     ──> ah-hub, ah-contracts
   ├─> ah-plugins-memory (真实记忆)       ──> ah-hub, ah-contracts
   ├─> ah-plugins-retrieval (知识库检索)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-security (安全检测)      ──> ah-hub, ah-contracts
   ├─> ah-plugins-subagent (子代理委派)    ──> ah-hub, ah-contracts
   ├─> ah-plugins-teams (多 agent 团队运行时) ──> ah-hub, ah-contracts
   ├─> ah-plugins-evolving (轨迹/评估/优化)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-rsi (递归自改进管线)     ──> ah-hub, ah-contracts
   ├─> ah-plugins-context (上下文压缩/offload) ──> ah-hub, ah-contracts
   ├─> ah-plugins-store (文件后端 KV/message) ──> ah-hub, ah-contracts
   ├─> ah-plugins-prompt (版本化模板注册表) ──> ah-hub, ah-contracts
   ├─> ah-plugins-queue (文件后端消息队列)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-workspace (清单/目标状态机) ──> ah-hub, ah-contracts
   ├─> ah-plugins-sandbox (策略沙箱 + rail)   ──> ah-hub, ah-contracts
   ├─> ah-plugins-code (python3 子进程执行)  ──> ah-hub, ah-contracts
   ├─> ah-plugins-web (ureq HTTP 客户端)     ──> ah-hub, ah-contracts
   ├─> ah-plugins-transport (A2A 风格 JSON-RPC) ──> ah-hub, ah-contracts
   ├─> ah-plugins-git (真实 git 子进程操作)   ──> ah-hub, ah-contracts
                                            └─dev-dep─> tokio process/time(测试)
                                            └─dev-dep─> ah-plugins-tools/sysop(测试)
                                            └─dev-dep─> ah-plugins-mock/tools/sysop/session-log(测试)
                                            └─dev-dep─> ah-plugins-mock/tools/sysop/session-log/subagent/evolving(测试)
                                            └─dev-dep─> ah-plugins-mock/tools/sysop/session-log(测试)
                                            └─dev-dep─> ah-plugins-mock/tools/sysop/session-log/subagent(测试)
   ├─> ah-plugins-mcp (真实 MCP stdio 传输) ──> ah-hub, ah-contracts
                                            └─dev-dep─> ah-plugins-tools(测试)
                                            (ah-plugins-mcp 另注入 mcp_call_tool 到 tools seam)
   ├─> ah-plugins-telemetry (真实 span 记录 + JSONL 导出) ──> ah-hub, ah-contracts
                                            └─dev-dep─> ah-plugins-tools(测试)
   ├─> ah-plugins-credentials (真实凭据引用:env provider) ──> ah-hub, ah-contracts
   ├─> ah-plugins-openai (真实 LLM HTTP)    ──> ah-hub, ah-contracts
                                            └─dev-dep─> ah-plugins-credentials(集成测试)
                                            (OpenAiPlugin::lazy 在 apply 时查 credentials seam)
```

依赖方向(单向、禁止环):

- ah-contracts:零依赖(仅 async-trait);
- ah-hub → ah-contracts;
- ah-plugins-* → ah-hub + ah-contracts(插件之间禁止互依赖);
- ah-app → ah-hub + 插件(组装)。

## 2. 规划:完整插件布局(源自 capability-map.md)

```text
ah-contracts                21 个 seam(见 capability-map.md §1)
ah-hub                      内核
ah-plugins-mock             仅 llm boot 桩(真实 provider 落地后移除)
ah-plugins-tools            真实工具注册表(已实现)
ah-plugins-sysop            真实本地 fs/shell 执行(已实现)
ah-plugins-openai            真实 LLM HTTP provider(已实现,需凭据 e2e)
ah-plugins-agent-loop        真实 ReAct 循环(已实现,注入 llm+tools)
ah-plugins-rails             真实 rails(ShellGuard 危险命令 / PathGuard 路径逃逸 / ToolBudget 调用上限)
ah-plugins-session-log       真实会话事件日志(已实现:JSONL 持久化 + 投影)
ah-plugins-core-*           common/application/runner/single-agent/context 等
ah-plugins-workflow-engine  工作流/图/controller/operator(迁移 rp301)
ah-plugins-session-log      会话事件日志(迁移 state.rs/persist.rs)
ah-plugins-sysop-*          fs/shell/code/sandbox(迁移 sys_operation.rs)
ah-plugins-harness-*        tools/rails/subagents/cli/workspace
ah-plugins-teams            团队运行时(内存 + SQLite 持久化,迁移 residual.rs)
ah-plugins-evolving         agent_evolving 域
ah-plugins-rsi              RSI + auto_harness
ah-plugins-store-*          redis/pulsar/gaussdb/elasticsearch/milvus/chroma
ah-plugins-mcp              真实 MCP stdio transport(已实现:子进程 + newline-delimited JSON-RPC 2.0)
ah-plugins-credentials     真实凭据引用(已实现:env provider,openai.api_key → OPENAI_API_KEY 等映射可配置;set/remove 显式报错)
ah-plugins-transport-*      a2a(规划)
ah-plugins-telemetry       telemetry(已实现:span 记录 + JSONL 导出;OTLP 导出留待后续,规划为 ah-plugins-otel)
ah-plugins-openai           第一个真实 LLM provider(迁移 OpenAiCompatibleClient;配置可从 credentials seam 解析,credentials 优先)
ah-plugins-devtools         dev_tools 域
ah-plugins-symphony         symphony
ah-app                      boot 入口 + demo + ah-cli(交互 CLI,消费会话管理)
```

## 3. 新增 crate 清单

1. 在 workspace Cargo.toml members + [workspace.dependencies] 登记;
2. 在本文件 §1/§2 更新依赖关系;
3. 在 capability-map.md 登记对应能力;
4. 遵循命名:插件为 ah-plugins-<域>[-<子域>],其余为 ah-<模块>;
5. CI 门禁会校验 fmt / clippy / test / mock 门禁。
