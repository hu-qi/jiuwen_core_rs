# agent-harness

Rust 原生 agent harness,以高解耦插件架构为目标,设计思路参考 DeepSeek Harness
(Cordis: 服务注册表 + 类型化事件 + 可逆注册)与 openJiuwen agent-core 的行为面。

## 架构方向

```text
agent-harness/
  crates/
    ah-contracts/     契约层: Seam trait + 事件契约 + 纯类型,零实现
    ah-hub/           插件内核: ServiceRegistry + EventBus + Plugin + Profile
    ah-plugins-*/     插件 crate(当前共 111 个 crate,其中约 108 个 ah-plugins-*)
    ah-app/           boot 入口: 读取 profile → 组装插件 → 解析 seam + ah-cli
  profiles/           组合配置: dev(mock,113 条) / prod(真实,112 条)
  docs/               文档集(架构/能力地图/目录/账本)
```

核心原则(对齐 DSH/Cordis):

- **无特权核心**:模型、工具、会话日志、agent 循环都是插件。
- **Seam 契约**:契约 crate 只声明接口(Service Definition / Provider / Consumer 三角),
  实现者依赖契约而非彼此。
- **类型化事件**:emit / serial / parallel / waterfall 四种分发。
- **可逆注册**:插件注册以 RAII guard(Effect)表达,卸载自动回滚。
- **日志即真相**:会话以 append-only 事件日志(JSONL)为唯一事实来源(已实现)。
- **Profile 门禁**:生产 profile 不允许出现 mock 插件,CI 校验展开后的插件清单。

## 当前框架能力(已实现,111 crates / 1284 tests / clippy 0 / fmt clean)

- **内核与契约**:ah-hub(ServiceRegistry + EventBus + Plugin/mount_all 拓扑挂载 + Profile)与
  ah-contracts(全部 seam 契约 + 类型化事件 + 服务键,零实现;数量以代码实测为准,见 docs/event-catalog.md)。
- **模型 provider**:ah-plugins-openai(OpenAI 兼容,credentials 解析 + SSE 流式)、
  ah-plugins-anthropic(Messages API,system 顶层 + tool_use/tool_result)。
- **核心 seam**:agent-loop(ReAct)、workflow(Start/End/LLM/Tool/Loop/SubWorkflow/Parallel +
  Http/Intent/Questioner + 检查点续跑 + LLM 节点流式消费)、pregel(超级步图)、
  subagent(隔离委派)、subagents(类型化 code/research/plan/verify)、session-log(JSONL 日志 +
  多会话)、context(预算组装/压缩/offload)、memory(JSON 记忆 + 工具)、graph-memory(知识图谱
  实体/关系/episode)、retrieval(BM25 + 确定性向量)、prompt(版本化模板)、queue(文件 + Redis 后端)、
  store(文件 + Redis + PostgreSQL 后端)、sandbox(策略沙箱)、code(python3 子进程)、web(HTTP)、
  transport(A2A JSON-RPC + SSE)、shell/fs/sysop(本地执行)。
- **harness**:tools(真实工具注册表 + pre/post-execute 管线)、rails(ShellGuard/PathGuard/
  ToolBudget/ApprovalRail/Security)、cli(Claude Code 风格渲染 + ah-cli 交互)、workspace(清单/目标)、
  credentials(env provider)、telemetry(span + JSONL + OTLP/JSON 导出)。
- **teams / evolving / rsi / dev_tools**:teams(SQLite 持久化 + swarmflow)、evolving(轨迹/评估/优化 +
  trajectory OTLP codec)、rsi(数据集生成(确定性 + LLM)+ 评测/精化/checkpoint + single_harness 候选门禁)、
  autoharness(六阶段)、rl(reward)、skill、tune、agentbuilder、symphony、controller、runner、
  operator(参数句柄)、optimizer(文本梯度)、trainer(训练循环)、external(外部 CLI agent)、
  oauth(设备码)、git、ci、mcp(stdio + http)。
- **mock 门禁**:dev profile 含 ah-plugins-mock(llm boot 桩);prod profile 112 个插件无 mock,CI 强制。
- ah-app:cargo run -p ah-app 从 profiles/dev.toml 启动;ah-cli 交互入口(/new /teams /rsi /queue 等子命令)。

## 构建与运行

```sh
cargo build --workspace
cargo test --workspace
cargo run -p ah-app
```
