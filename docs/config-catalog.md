# 配置目录(config-catalog.md)

> 等价 DSH 的 config-catalog:登记 profile 与插件的全部配置字段。
> 环境相关的可调参数必须是配置字段(来自 profile),不允许硬编码在插件里。
> 本文档按 crates/ah-app/src/lib.rs::plugin_catalog 与 profiles/*.toml 实际代码审计(2025-08 核对)。

## 1. Profile(profiles/*.toml)

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| name | string | 是 | profile 名(诊断显示) |
| bundles | array | 否 | bundle 列表,按顺序堆叠 |
| bundles[].id | string | 是 | bundle 标识 |
| bundles[].plugins | string[] | 否 | 插件名清单,去重展开 |

当前文件(实测插件数):

| 文件 | name | bundle | 插件数 | 要点 |
| --- | --- | --- | --- | --- |
| profiles/dev.toml | dev | boot | 51 | 含 ah-plugins-mock(llm boot 桩);文件后端 store/queue;无真实 LLM provider |
| profiles/prod.toml | prod | real | 51 | 无 mock(全部真实实现);Redis 后端 store-redis/queue-redis;ah-plugins-openai 真实 LLM |

dev 与 prod 差异:dev 独有 ah-plugins-mock、ah-plugins-store、ah-plugins-queue;prod 独有
ah-plugins-openai、ah-plugins-store-redis、ah-plugins-queue-redis。

## 2. 插件注册目录(ah-app)

| 字段 | 说明 |
| --- | --- |
| 插件名(profile 引用) | 与 Plugin::name() 一致,如 ah-plugins-mock |
| 插件对象 | 目录中名称 → Arc<dyn Plugin> 映射,未来可扩展为动态注册 |

### 2.1 插件清单(58 个,plugin_catalog 全量;dev/prod 各引用 51 个)

| 插件 | 配置/构造参数(plugin_catalog 实际传入) | dev | prod | 说明 |
| --- | --- | --- | --- | --- |
| ah-plugins-mock | 无 | ✓ | | llm boot 桩(dev only) |
| ah-plugins-credentials | 无目录参数;CredentialsPlugin::default()(默认映射见 §3) | ✓ | ✓ | 真实凭据引用,env provider 只读 |
| ah-plugins-tools | 无 | ✓ | ✓ | 工具注册表 + 执行管线 |
| ah-plugins-sysop | workspace_root | ✓ | ✓ | 真实 fs/shell 工具 |
| ah-plugins-rails | 无 | ✓ | ✓ | ShellGuard(危险命令) |
| ah-plugins-rails-path | 无 | ✓ | ✓ | PathGuard(路径逃逸) |
| ah-plugins-rails-budget | ToolBudgetRailPlugin::new(100)(目录硬编码 100) | ✓ | ✓ | 工具调用上限 |
| ah-plugins-rails-approval | workspace_root/approvals + 14 个工具名白名单 | | | 审批 rail(目录中有,未进 dev/prod) |
| ah-plugins-security | 无 | ✓ | ✓ | guardrails + pre-execute rail |
| ah-plugins-subagent | 无(注入 llm/tools/sessions/manager) | ✓ | ✓ | 隔离会话委派 + context 消费 |
| ah-plugins-teams | 无 | | | 内存团队运行时(测试;未进 dev/prod) |
| ah-plugins-teams-sqlite | workspace_root/teams.db | ✓ | ✓ | SQLite 团队运行时 + queue 消息 |
| ah-plugins-teams-workflow | workspace_root/swarm-journals | ✓ | ✓ | swarmflow 编排(phase/agent/预算/journal 续跑) |
| ah-plugins-evolving | workspace_root/evolving(注入 llm/session-manager) | ✓ | ✓ | 轨迹抽取/评估/优化;经验 experiences.jsonl |
| ah-plugins-rsi | workspace_root/rsi | ✓ | ✓ | RSI 管线 + run_rounds + checkpoint |
| ah-plugins-rsi-analyzer | 无 | ✓ | ✓ | 评测结果分析(信号/归因/artifact) |
| ah-plugins-rsi-single-harness | workspace_root/single-harness | ✓ | ✓ | 单 harness 迭代编排 + holdout 门禁 + 续跑 |
| ah-plugins-context | workspace_root/context | ✓ | ✓ | 上下文预算/压缩/offload |
| ah-plugins-controller | 无 | ✓ | ✓ | 任务生命周期/优先级/层级/调度 |
| ah-plugins-store | workspace_root/store | ✓ | | KV + message 文件后端 |
| ah-plugins-store-redis | RedisStorePlugin::new("redis://127.0.0.1:6379/")(目录硬编码) | | ✓ | Redis KV 后端(需本地 Redis) |
| ah-plugins-store-pg | PgStorePlugin::new("postgres://postgres:ah@127.0.0.1:64329/ah")(目录硬编码) | | | PostgreSQL 后端(目录中有,未进 dev/prod) |
| ah-plugins-prompt | workspace_root/prompts | ✓ | ✓ | 版本化模板注册表({{var}} 渲染) |
| ah-plugins-queue | workspace_root/queue | ✓ | | 日志+游标消息队列(文件后端) |
| ah-plugins-queue-redis | RedisQueuePlugin::new("redis://127.0.0.1:6379/")(目录硬编码,key 前缀 ah:q) | | ✓ | Redis 消息队列(需本地 Redis) |
| ah-plugins-workspace | workspace_root | ✓ | ✓ | workspace.json + 目标状态机 |
| ah-plugins-sandbox | workspace_root/sandbox | ✓ | ✓ | sandbox.json 策略 + rail |
| ah-plugins-sandbox-rail | 无(注入 sandbox) | ✓ | ✓ | pre-execute 策略执行 |
| ah-plugins-code | workspace_root/scratch | ✓ | ✓ | python3 执行 + run_code 工具 |
| ah-plugins-web | 无 | ✓ | ✓ | ureq HTTP + web_fetch 工具 |
| ah-plugins-transport | AgentCard{name=agent-harness,url=http://127.0.0.1:0/,skills=[agent]}(目录构造) | ✓ | ✓ | A2A 风格 JSON-RPC 传输 |
| ah-plugins-git | 无 | ✓ | ✓ | 本地 git 操作 |
| ah-plugins-graph-memory | workspace_root/graph-memory | ✓ | ✓ | 知识图谱记忆(实体/关系/episode + JSONL) |
| ah-plugins-ci | 无 | ✓ | ✓ | CI gate 运行器(子进程门禁) |
| ah-plugins-cli | 无 | ✓ | ✓ | 终端渲染(工具调用/结果/todo checkbox) |
| ah-plugins-external | CliAgentAdapter::generic_streaming("__DONE__")(目录构造) | ✓ | ✓ | 外部 CLI agent 运行时(流式 stdin + 单发 argv) |
| ah-plugins-autoharness | 无 | ✓ | ✓ | 自动改进周期(assess→publish) |
| ah-plugins-rl | 无 | ✓ | ✓ | RL 奖励函数 |
| ah-plugins-runner | 无 | ✓ | ✓ | 回调链(优先级/retry/timeout/rollback + 指标) |
| ah-plugins-subagents | 无 | ✓ | ✓ | 类型化子代理(code/research/plan/verify + 白名单) |
| ah-plugins-skill | workspace_root/skills | ✓ | ✓ | 技能注册 + 评估(subagent 委派 + evolving 轨迹) |
| ah-plugins-pregel | 无 | ✓ | ✓ | 超级步图引擎(状态通道/条件触发/中断) |
| ah-plugins-tune | 无 | ✓ | ✓ | 训练流水线(subagent 执行 + evolving 评估/优化) |
| ah-plugins-trainer | 无 | ✓ | ✓ | 自进化训练循环(基线 → 前向/更新/验证 → checkpoint) |
| ah-plugins-oauth | 无 | ✓ | ✓ | 设备码 OAuth 客户端 |
| ah-plugins-operator | 无 | ✓ | ✓ | 自进化算子(参数句柄 + freeze + 回调) |
| ah-plugins-optimizer | 无 | ✓ | ✓ | 文本梯度优化器(评估 → 算子参数更新) |
| ah-plugins-agentbuilder | 无 | ✓ | ✓ | agent 构建(NL→设计→DSL→工作流执行) |
| ah-plugins-symphony | workspace_root/symphony | ✓ | ✓ | 能力注册/指纹/检索/编排/执行 |
| ah-plugins-session-log | session_path(默认会话 JSONL)+ session_dir(多会话目录) | ✓ | ✓ | JSONL 会话 + checkpoint/restore |
| ah-plugins-workflow | 无 | ✓ | ✓ | 工作流引擎 |
| ah-plugins-memory | memory_dir(boot 注入) | ✓ | ✓ | JSON 文件记忆(dir/{key}.json) |
| ah-plugins-retrieval | retrieval_dir(boot 注入) | ✓ | ✓ | BM25 + 本地向量(dir/{doc_id}.json) |
| ah-plugins-mcp | McpPlugin::new("npx", ["-y", "@modelcontextprotocol/server-everything"])(目录硬编码) | ✓ | ✓ | MCP stdio 客户端;懒 spawn |
| ah-plugins-telemetry | telemetry_dir(boot 注入) | ✓ | ✓ | span 记录 + JSONL 导出 |
| ah-plugins-agent-loop | AgentLoopPlugin::default()(max_iterations=8,目录硬编码) | ✓ | ✓ | ReAct 循环(日志驱动) |
| ah-plugins-openai | OpenAiPlugin::lazy()(apply 时解析,见 §3) | | ✓ | 真实 LLM provider(prod) |
| ah-plugins-anthropic | AnthropicPlugin::lazy()(apply 时解析,见 §3) | | | 真实 LLM provider(目录中有,未进 dev/prod) |

## 3. 已实现配置字段(按插件)

插件引入可调参数时,必须在 profile 中声明配置字段并在本表登记。

| 插件 | 配置字段 | 说明 |
| --- | --- | --- |
| ah-plugins-openai(已实现) | base_url / api_key / model / timeout;来源:credentials seam(openai.api_key / openai.base_url / openai.model,优先)+ OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_MODEL(兜底);默认 base_url https://api.openai.com/v1、model gpt-4o-mini、timeout 60s | 真实 LLM provider;credentials 与 env 都无 key 时挂载显式失败(不静默降级) |
| ah-plugins-anthropic(已实现) | api_key / base_url / model / timeout;仅环境变量:ANTHROPIC_API_KEY(必须)、ANTHROPIC_BASE_URL(默认 https://api.anthropic.com)、ANTHROPIC_MODEL(默认 claude-3-5-sonnet)、timeout 60s | 真实 LLM provider;缺 ANTHROPIC_API_KEY 时 apply 显式失败;不读 credentials seam |
| ah-plugins-credentials(已实现) | 凭据名 → 环境变量名映射,默认 openai.api_key → OPENAI_API_KEY / openai.base_url → OPENAI_BASE_URL / openai.model → OPENAI_MODEL;可用 CredentialsPlugin::new / EnvCredentialProvider::new 注入 | 真实环境变量 provider;env 只读,set/remove 显式报错;凭据名是稳定契约(如 openai.api_key),profile 不直接写凭据值 |
| ah-plugins-mcp(已实现,stdio) | command / args(默认 npx -y @modelcontextprotocol/server-everything;经 McpPlugin::new 注入) | 真实 MCP stdio transport;懒 spawn 子进程,首次调用时建立协议连接;http 传输规划 |
| ah-plugins-store-redis(已实现) | url(目录硬编码 redis://127.0.0.1:6379/,RedisStorePlugin::new) | Redis KV 后端,与文件后端同一 seam 可互换;prod profile 使用 |
| ah-plugins-queue-redis(已实现) | url(目录硬编码 redis://127.0.0.1:6379/,RedisQueuePlugin::new);key 前缀 ah:q | Redis 消息队列(日志 LIST + 序号 INCR + 游标 STRING);prod profile 使用 |
| ah-plugins-store-pg(已实现) | url(目录硬编码 postgres://postgres:ah@127.0.0.1:64329/ah,PgStorePlugin::new) | PostgreSQL 后端(KV + message 两表);目录中有,未进 dev/prod |

环境变量清单(grep OPENAI_/ANTHROPIC_ 实测):OPENAI_API_KEY / OPENAI_BASE_URL / OPENAI_MODEL、
ANTHROPIC_API_KEY / ANTHROPIC_BASE_URL / ANTHROPIC_MODEL;credentials 插件默认映射指向同一组
OPENAI_* 变量。除此之外无其他环境变量配置。

现状说明(审计备注):rails-budget 上限 100、store-redis / queue-redis / store-pg 的 URL、
transport 的 AgentCard、mcp 的 command/args、external 的 "__DONE__" 结束符、agent-loop 的
max_iterations=8 目前均由 ah-app::plugin_catalog 直接构造传入(硬编码,未走 profile 配置字段)。
按 §4 纪律第 1 条,部署环境差异应逐步配置化。

## 4. 配置纪律

1. 部署环境差异(地址、凭据、超时、预算)必须是配置字段,不是 DEFAULT_* 常量;
2. 协议常量、外部规范、安全不变量保持固定,不进配置;
3. 配置错误在加载时显式报错(fail loud),不静默跳过;
4. 凭据引用(如 api_key_ref)指向 credentials seam,不直接写入 profile。
