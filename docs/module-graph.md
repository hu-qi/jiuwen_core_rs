# 模块依赖图(module-graph.md)

> 等价 DSH 的 module-graph:登记 crate 布局与依赖规则。新增 crate 时必须更新本文件。

## 1. 当前 crate 依赖(全部 107 crates)

```text
┌──────────────────────────────────────────────────────────────────────┐
│ ah-app(组装层:装配全部 47 个插件;bin:main demo / ah-cli 交互 CLI)        │
└───────────────────────────┬──────────────────────────────────────────┘
                            │ 运行时依赖:ah-hub + ah-contracts + 全部 47 个插件
                            ▼
┌──────────────────────────────────────────────────────────────────────┐
│ ah-hub(内核:Context / EventBus / Plugin 生命周期;仅依赖 ah-contracts)   │
└───────────────────────────┬──────────────────────────────────────────┘
                            │
                            ▼
┌──────────────────────────────────────────────────────────────────────┐
│ ah-contracts(契约:seam / 事件 / effect / 类型;零 ah-* 依赖)             │
└──────────────────────────────────────────────────────────────────────┘

47 个插件:运行时仅依赖 ah-hub + ah-contracts(插件间零运行时互依赖,
测试经 dev-dependencies 引入其他插件);统一标注 ──> ah-hub, ah-contracts。

[核心循环 / 代理]
├─> ah-plugins-agent-loop   (真实 ReAct 循环,注入 llm+tools+session)   ──> ah-hub, ah-contracts
├─> ah-plugins-agentbuilder (NL 任务 → 设计 → 工作流 DSL → 真实执行)     ──> ah-hub, ah-contracts
├─> ah-plugins-subagent     (真实子代理:隔离会话 + 预算受限循环)         ──> ah-hub, ah-contracts
├─> ah-plugins-subagents    (类型化子代理 code/research/plan/verify)    ──> ah-hub, ah-contracts
├─> ah-plugins-workflow     (真实工作流引擎:Start/End/LLM/Tool/Loop 节点) ──> ah-hub, ah-contracts
├─> ah-plugins-pregel       (真实 Pregel 超级步图引擎)                  ──> ah-hub, ah-contracts
├─> ah-plugins-controller   (TaskManager 任务 CRUD/状态迁移/父子层级)     ──> ah-hub, ah-contracts
├─> ah-plugins-operator     (自进化算子 LLMCall 等)                     ──> ah-hub, ah-contracts
├─> ah-plugins-optimizer    (文本梯度优化器 backward)                   ──> ah-hub, ah-contracts
├─> ah-plugins-trainer      (自进化训练器:验证基线评估)                  ──> ah-hub, ah-contracts
├─> ah-plugins-tune         (训练流水线:subagent 委派 + evolving 评估)   ──> ah-hub, ah-contracts
├─> ah-plugins-runner       (回调链:优先级降序执行)                      ──> ah-hub, ah-contracts
├─> ah-plugins-rl           (RL 奖励函数;VERL/PPO/LoRA 留待后续)         ──> ah-hub, ah-contracts
├─> ah-plugins-rsi          (递归自改进:数据集生成/评估/优化)            ──> ah-hub, ah-contracts
├─> ah-plugins-evolving     (轨迹抽取/评估/优化运行时)                   ──> ah-hub, ah-contracts
├─> ah-plugins-autoharness  (assess→plan→implement→verify→commit→publish) ──> ah-hub, ah-contracts
├─> ah-plugins-symphony     (能力注册 + 语义指纹 + 编排计划 + 执行)        ──> ah-hub, ah-contracts
├─> ah-plugins-teams        (多 agent 团队任务板 + swarm 工作流)         ──> ah-hub, ah-contracts
├─> ah-plugins-external     (第三方 CLI 进程作为团队成员)                ──> ah-hub, ah-contracts
├─> ah-plugins-skill        (技能注册与评估,subagent 真实委派)           ──> ah-hub, ah-contracts
├─> ah-plugins-cli          (Claude Code 风格终端渲染)                  ──> ah-hub, ah-contracts
├─> ah-plugins-team-prompts (team.plan 双语提示模板 + bridge brief/名册概览,对齐 agent_teams/prompts) ──> ah-hub, ah-contracts
├─> ah-plugins-team-schema  (ssh transport 校验 + 任务图模型 + infra transport/storage 注册表 + blueprint 装配校验,对齐 agent_teams/schema) ──> ah-hub, ah-contracts

[模型 provider]
├─> ah-plugins-mock         (仅 llm boot 桩,真实 provider 落地后移除)   ──> ah-hub, ah-contracts
├─> ah-plugins-openai       (真实 OpenAI 兼容 HTTP provider;lazy 查 credentials seam) ──> ah-hub, ah-contracts
├─> ah-plugins-anthropic    (真实 Anthropic Messages API provider)      ──> ah-hub, ah-contracts
├─> ah-plugins-oauth        (真实设备码 OAuth 客户端)                   ──> ah-hub, ah-contracts
├─> ah-plugins-credentials  (真实凭据引用:env provider)                 ──> ah-hub, ah-contracts

[工具与执行]
├─> ah-plugins-tools        (真实工具注册表,pre/post-execute 管线)      ──> ah-hub, ah-contracts
├─> ah-plugins-manifest     (真实 harness 元素 manifest:描述符目录/工厂注册表/kind 路由注册) ──> ah-hub, ah-contracts
├─> ah-plugins-sysop        (真实受限 fs/shell + read_file/run_shell 工具) ──> ah-hub, ah-contracts
├─> ah-plugins-code         (python3 子进程执行)                        ──> ah-hub, ah-contracts
├─> ah-plugins-sandbox      (策略沙箱 + tools/pre-execute rail)         ──> ah-hub, ah-contracts
├─> ah-plugins-rails        (ShellGuard/PathGuard/ToolBudget/Approval rail) ──> ah-hub, ah-contracts
├─> ah-plugins-security     (规则后端 guardrails + 通用 pre-execute rail) ──> ah-hub, ah-contracts
├─> ah-plugins-mcp          (真实 MCP stdio transport;注入 mcp_call_tool 到 tools seam) ──> ah-hub, ah-contracts
├─> ah-plugins-web          (ureq HTTP 客户端)                          ──> ah-hub, ah-contracts
├─> ah-plugins-transport    (A2A 风格 JSON-RPC over HTTP)               ──> ah-hub, ah-contracts
├─> ah-plugins-a2a          (A2A 协议转换/AgentCard 适配/客户端聚合/URL 归一化,对齐 extensions/a2a) ──> ah-hub, ah-contracts
├─> ah-plugins-git          (真实 git 子进程操作)                        ──> ah-hub, ah-contracts
├─> ah-plugins-ci           (子进程 CI gate 运行器)                      ──> ah-hub, ah-contracts

[记忆与数据]
├─> ah-plugins-session-log  (JSONL 会话日志,投影 derive_messages)        ──> ah-hub, ah-contracts
├─> ah-plugins-memory       (持久化记忆,remember/recall/forget 工具)     ──> ah-hub, ah-contracts
├─> ah-plugins-graph-memory (知识图谱记忆:实体抽取 + 邻接)               ──> ah-hub, ah-contracts
├─> ah-plugins-retrieval    (本地知识库检索:BM25 + 哈希 n-gram 向量)      ──> ah-hub, ah-contracts
├─> ah-plugins-store        (文件后端 KV/message store)                 ──> ah-hub, ah-contracts
├─> ah-plugins-queue        (文件后端消息队列,channel JSONL)             ──> ah-hub, ah-contracts
├─> ah-plugins-prompt       (版本化模板注册表,{{var}} 渲染)              ──> ah-hub, ah-contracts
├─> ah-plugins-context      (上下文引擎:token 预算组装/压缩)             ──> ah-hub, ah-contracts
├─> ah-plugins-workspace    (工作区清单/目标状态机)                      ──> ah-hub, ah-contracts

[遥测]
└─> ah-plugins-telemetry    (span 记录 + JSONL 导出;监听 agent/step + tools/post-execute) ──> ah-hub, ah-contracts
```

依赖方向(单向、禁止环):

- ah-contracts:零 ah-* 依赖(仅 async-trait / serde);
- ah-hub → ah-contracts;
- ah-plugins-* → ah-hub + ah-contracts(插件之间运行时禁止互依赖;测试经 dev-dependencies);
- ah-app → ah-hub + 全部 47 个插件(组装)。

## 2. 规划:完整插件布局(源自 capability-map.md)

```text
ah-contracts                契约层:seam / 事件 / effect / 类型(107 crates 之一)
ah-hub                      内核:Context / EventBus / Plugin 生命周期
ah-app                      boot 入口 + demo(main)+ ah-cli(交互 CLI,消费会话管理)

[核心循环 / 代理](全部已实现)
ah-plugins-agent-loop        (已实现)真实 ReAct 循环,注入 llm+tools+session
ah-plugins-agentbuilder      (已实现)NL 任务 → 设计 → 工作流 DSL → 真实执行
ah-plugins-subagent          (已实现)子代理委派:隔离会话 + 预算受限循环
ah-plugins-subagents         (已实现)类型化子代理 code/research/plan/verify + 工具白名单
ah-plugins-workflow          (已实现)工作流引擎(Start/End/LLM/Tool/Loop + 条件边)
ah-plugins-pregel            (已实现)Pregel 风格超级步图引擎
ah-plugins-controller        (已实现)TaskManager:任务 CRUD/状态迁移/父子层级(防环)
ah-plugins-operator          (已实现)自进化算子(LLMCall 等)
ah-plugins-optimizer         (已实现)文本梯度优化器(backward)
ah-plugins-trainer           (已实现)自进化训练器(验证基线评估)
ah-plugins-tune              (已实现)训练流水线(subagent 委派 + evolving 评估/优化)
ah-plugins-runner            (已实现)回调链(优先级降序)
ah-plugins-rl                (已实现)RL 奖励函数(VERL/PPO/LoRA 留待后续)
ah-plugins-rsi               (已实现)递归自改进管线(数据集生成/评估/优化)
ah-plugins-evolving          (已实现)轨迹抽取/评估/优化运行时
ah-plugins-autoharness       (已实现)auto_harness 编排(assess→plan→implement→verify→commit→publish)
ah-plugins-symphony          (已实现)能力注册 + 语义指纹 + 可解释编排 + 执行
ah-plugins-teams             (已实现)团队任务板 + review/settle + swarm 工作流(内存 + SQLite)
ah-plugins-external          (已实现)外部 CLI agent 运行时(第三方进程作为团队成员)
ah-plugins-skill             (已实现)技能注册与评估(subagent 真实委派)
ah-plugins-cli               (已实现)Claude Code 风格终端渲染

[模型 provider](全部已实现)
ah-plugins-mock              (已实现)仅 llm boot 桩(真实 provider 落地后移除)
ah-plugins-openai            (已实现)真实 OpenAI 兼容 HTTP provider;凭据经 credentials seam
ah-plugins-anthropic         (已实现)真实 Anthropic Messages API provider
ah-plugins-oauth             (已实现)设备码 OAuth 客户端
ah-plugins-credentials       (已实现)env provider 凭据引用(openai.api_key → OPENAI_API_KEY 映射可配置)

[工具与执行](全部已实现)
ah-plugins-tools             (已实现)真实工具注册表 + pre/post-execute 管线
ah-plugins-manifest          (已实现)真实 harness 元素 manifest:描述符目录/工厂注册表/kind 路由注册
ah-plugins-resources         (已实现)真实扩展资源:Spec 模型/MCP 归一化/模板渲染/路径校验/ExtensionParts 解析
ah-plugins-lsp               (已实现)真实 LSP 子系统:状态机/诊断注册表/5 语言 server 配置
ah-plugins-kv-cache          (已实现)真实 KV-cache 策略钩子:affinity/sticky 判定 + prefetch/offload/evict 信号
ah-plugins-worktree          (已实现)真实团队 worktree 确定性部分:命名(slug+sha256)/成员状态归属判定
ah-plugins-checkpointer      (已实现)真实 Redis checkpointer:TTL/key 构造/四存储/生命周期钩子(RedisStore seam 注入)
ah-plugins-prompt-builder-devtools (已实现)真实 dev_tools 提示构建器:badcase/feedback/meta-template(LLM seam 注入)
ah-plugins-skill-creator     (已实现)真实技能创建流水线:slugify/资产编号/过滤/去幻影图片(抓取与 LLM seam 注入)
ah-plugins-sysop             (已实现)受限 fs/shell + read_file/write_file/list_dir/run_shell 工具
ah-plugins-code              (已实现)python3 子进程代码执行
ah-plugins-sandbox           (已实现)策略沙箱 + tools/pre-execute rail
ah-plugins-rails             (已实现)ShellGuard / PathGuard / ToolBudget / Approval rail
ah-plugins-security          (已实现)安全检测(提示注入/敏感数据)+ 通用 rail
ah-plugins-mcp               (已实现)MCP stdio transport(子进程 + newline-delimited JSON-RPC 2.0),注入 mcp_call_tool
ah-plugins-web               (已实现)ureq HTTP 客户端
ah-plugins-transport         (已实现)A2A 风格 JSON-RPC over HTTP
ah-plugins-git               (已实现)真实 git 子进程操作(init/status/add/commit/log/diff_stat/branch)
ah-plugins-ci                (已实现)子进程 CI gate 运行器(超时强杀)

[记忆与数据](全部已实现)
ah-plugins-session-log       (已实现)JSONL 会话日志 + 投影 + 广播 session/event
ah-plugins-memory            (已实现)持久化记忆 + remember/recall/forget 工具
ah-plugins-graph-memory      (已实现)知识图谱记忆(实体抽取 + 邻接)
ah-plugins-retrieval         (已实现)本地知识库检索(BM25 + 哈希 n-gram 向量)
ah-plugins-store             (已实现)文件后端 KV/message store
ah-plugins-queue             (已实现)文件后端消息队列(channel JSONL + 消费游标)
ah-plugins-prompt            (已实现)版本化模板注册表({{var}} 渲染)
ah-plugins-context           (已实现)上下文引擎(token 预算组装/压缩/offload)
ah-plugins-workspace         (已实现)工作区清单 + 目标状态机(create_goal / update_goal_status)

[遥测](已实现)
ah-plugins-telemetry         (已实现)span 记录 + JSONL 导出(监听 agent/step + tools/post-execute;OTLP 留待后续,规划为 ah-plugins-otel)

[规划(未落地 crate)]
ah-plugins-single-harness    single-harness 域(seam 已定义于 ah-contracts/src/single_harness.rs,插件待落地)
ah-plugins-store-*           redis / pulsar / gaussdb / elasticsearch / milvus / chroma 后端
ah-plugins-transport-*       a2a 完整实现(当前 ah-plugins-transport 已落地基础 JSON-RPC)
ah-plugins-otel              OTLP 导出器
ah-plugins-devtools          dev_tools 域
```

## 3. 新增 crate 清单

1. 在 workspace Cargo.toml members + [workspace.dependencies] 登记;
2. 在本文件 §1/§2 更新依赖关系;
3. 在 capability-map.md 登记对应能力;
4. 遵循命名:插件为 ah-plugins-<域>[-<子域>],其余为 ah-<模块>;
5. CI 门禁会校验 fmt / clippy / test / mock 门禁。
