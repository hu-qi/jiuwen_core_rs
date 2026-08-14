# 配置目录(config-catalog.md)

> 等价 DSH 的 config-catalog:登记 profile 与插件的全部配置字段。
> 环境相关的可调参数必须是配置字段(来自 profile),不允许硬编码在插件里。

## 1. Profile(profiles/*.toml)

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| name | string | 是 | profile 名(诊断显示) |
| bundles | array | 否 | bundle 列表,按顺序堆叠 |
| bundles[].id | string | 是 | bundle 标识 |
| bundles[].plugins | string[] | 否 | 插件名清单,去重展开 |

当前文件:profiles/dev.toml(name=dev,bundles=[{id=mock, plugins=[ah-plugins-mock]}])。

## 2. 插件注册目录(ah-app)

| 字段 | 说明 |
| --- | --- |
| 插件名(profile 引用) | 与 Plugin::name() 一致,如 ah-plugins-mock |
| 插件对象 | 目录中名称 → Arc<dyn Plugin> 映射,未来可扩展为动态注册 |

### 2.1 插件清单(29 个,dev/prod profile 全部引用)

| 插件 | 配置/构造参数 | 说明 |
| --- | --- | --- |
| ah-plugins-mock | 无 | llm boot 桩(dev only) |
| ah-plugins-tools | 无 | 工具注册表 + 执行管线 |
| ah-plugins-sysop | workspace_root | 真实 fs/shell 工具 |
| ah-plugins-rails | 无 | ShellGuard(危险命令) |
| ah-plugins-rails-path | 无 | PathGuard(路径逃逸) |
| ah-plugins-rails-budget | max_calls(目录默认 100) | 工具调用上限 |
| ah-plugins-security | 无 | guardrails + pre-execute rail |
| ah-plugins-session-log | session_path / session_dir | JSONL 会话 + checkpoint/restore |
| ah-plugins-workflow | 无 | 工作流引擎 |
| ah-plugins-memory | memory_dir | JSON 文件记忆 |
| ah-plugins-retrieval | retrieval_dir | BM25 + 本地向量 |
| ah-plugins-subagent | 无(注入 llm/tools/sessions/manager) | 隔离会话委派 + context 消费 |
| ah-plugins-teams-sqlite | db_path(目录默认 workspace/teams.db) | SQLite 团队运行时 + queue 消息 |
| ah-plugins-teams | 无 | 内存团队运行时(测试) |
| ah-plugins-evolving | 无(注入 llm/session-manager) | 轨迹/评估/优化 |
| ah-plugins-rsi | dir(目录默认 workspace/rsi) | RSI 管线 + run_rounds |
| ah-plugins-context | offload_dir(目录默认 workspace/context) | 上下文压缩;agent-loop/subagent 消费 |
| ah-plugins-store | dir(目录默认 workspace/store) | KV + message 文件后端 |
| ah-plugins-prompt | dir(目录默认 workspace/prompts) | 版本化模板注册表 |
| ah-plugins-queue | dir(目录默认 workspace/queue) | 日志+游标消息队列 |
| ah-plugins-workspace | dir(目录默认 workspace_root) | workspace.json + 目标状态机 |
| ah-plugins-sandbox | dir(目录默认 workspace/sandbox) | sandbox.json 策略 + rail |
| ah-plugins-sandbox-rail | 无(注入 sandbox) | pre-execute 策略执行 |
| ah-plugins-code | dir(目录默认 workspace/scratch) | python3 执行 + run_code 工具 |
| ah-plugins-web | 无 | ureq HTTP + web_fetch 工具 |
| ah-plugins-transport | card(目录默认注入本端点卡) | A2A 风格 JSON-RPC 传输 |
| ah-plugins-telemetry | telemetry_dir | span 记录 + JSONL 导出 |
| ah-plugins-agent-loop | max_iterations(默认 8) | ReAct 循环 |
| ah-plugins-openai | 见 §3 | 真实 LLM provider |
| ah-plugins-mcp | command/args(见 §3) | MCP stdio transport |

## 3. 规划:每插件配置

插件引入可调参数时,必须在 profile 中声明配置字段并在本表登记:

| 插件 | 配置字段 | 说明 |
| --- | --- | --- |
| ah-plugins-openai(已实现) | base_url / api_key / model / timeout,来源:credentials seam(openai.base_url / openai.api_key / openai.model,优先)+ OPENAI_BASE_URL / OPENAI_API_KEY / OPENAI_MODEL(兜底) | 真实 LLM provider;两者都无 key 时挂载显式失败(不静默降级) |
| ah-plugins-credentials(已实现) | 凭据名 → 环境变量名映射,默认 openai.api_key → OPENAI_API_KEY / openai.base_url → OPENAI_BASE_URL / openai.model → OPENAI_MODEL;可用 CredentialsPlugin::new / EnvCredentialProvider::new 注入 | 真实环境变量 provider;env 只读,set/remove 显式报错;凭据名是稳定契约(如 openai.api_key),profile 不直接写凭据值 |
| ah-plugins-redis | url / ttl / namespace | 检查点与 KV |
| ah-plugins-pulsar | url / topic / subscription | 消息队列 |
| ah-plugins-elasticsearch | url / index / auth | 向量库 |
| ah-plugins-mcp(已实现,stdio) | command / args(默认 npx -y @modelcontextprotocol/server-everything;经 McpPlugin::new 注入) | 真实 MCP stdio transport;懒 spawn 子进程,首次调用时建立协议连接;http 传输规划 |

## 4. 配置纪律

1. 部署环境差异(地址、凭据、超时、预算)必须是配置字段,不是 DEFAULT_* 常量;
2. 协议常量、外部规范、安全不变量保持固定,不进配置;
3. 配置错误在加载时显式报错(fail loud),不静默跳过;
4. 凭据引用(如 api_key_ref)指向 credentials seam,不直接写入 profile。