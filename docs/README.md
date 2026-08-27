# agent-harness 文档集

本文档集参考 DeepSeek Harness 的文档体系制定,目标有二:

1. **完整贯彻 DSH 架构理念**:无特权核心、Seam 契约、类型化事件、可逆注册、
   Profile 组合、日志即真相、mock 门禁;
2. **以完整实现 agent-core(Python)全部功能为目标**:能力地图将 Python 各域
   逐模块映射到 Rust 的 seam / 插件 / 工作包,作为开发与验收的单一依据。

## 文档清单与阅读顺序

### 必读(改动任何代码之前)

| 文档 | 角色 | 读者 |
| --- | --- | --- |
| [architecture.md](architecture.md) | 架构约束:理念、分层、硬性规则 | 所有贡献者 |
| [hub-primer.md](hub-primer.md) | 内核语义入门:注册表/事件/Effect/Plugin/Profile | 写插件或改内核者 |
| [capability-map.md](capability-map.md) | 能力地图:agent-core 全功能 → seam/插件/工作包 | 规划与验收 |
| [parity-audit.md](parity-audit.md) | 对等审计基线:7 域 97 子模块 done/partial/missing + file:line 证据 | 规划与验收 |
| [agent-guide.md](agent-guide.md) | 生成参考:面向 AI 代理的任务配方与验证命令 | 编码代理 |

### 参考(按需)

| 文档 | 角色 |
| --- | --- |
| [development.md](development.md) | 开发流程:工作包生命周期、CI 门禁、提交规范 |
| [testing.md](testing.md) | 测试与对等验证:契约 fixtures、差分契约、覆盖率 |
| [usage.md](usage.md) | 用户文档:插件编写、seam 定义、事件、profile |
| [glossary.md](glossary.md) | 术语表:全文档集统一术语 |
| [event-catalog.md](event-catalog.md) | 事件目录:事件 × 模式 × 生产者 × 消费者 |
| [config-catalog.md](config-catalog.md) | 配置目录:profile 与插件配置字段 |
| [module-graph.md](module-graph.md) | 模块依赖图:crate 布局与新增规则 |

## 规划文档(能力落地后再写)

以下文档原定描述尚未实现的子系统,为避免"文档先于代码"的空转,只在对应能力
达到 partial 后创建并登记。当前各触发条件均已满足(子系统已实现),除
persistence-catalog.md 外均尚未创建(文档滞后于代码);如需补写,应按子系统
现状(capability-map.md)记录实现,而非"规划"。

| 规划文档 | 触发条件 | 内容 | 状态 |
| --- | --- | --- | --- |
| tool-catalog.md | tools seam 落地 | 工具注册表、工具分类 | 未创建:tools 已落地,内容分散于 usage.md 与 capability-map §2.2,文档未跟进 |
| tool-execution-pipeline.md | 工具执行管线 | 执行管线、鉴权、超时、回滚 | 未创建:管线已落地(pre/post-execute waterfall + ApprovalRail),文档未跟进 |
| agent-lifecycle.md | agent-loop 落地 | turn/step 生命周期、事件序列 | 未创建:agent-loop 已落地,事件序列见 event-catalog.md,文档未跟进 |
| persistence-catalog.md | session log 落地 | 持久化格式与版本策略 | ✅ 已创建(session log 已落地) |
| cookbook.md | 插件数 > 3 | 面向插件作者的扩展配方集 | 未创建:插件 crate 已 47 个,文档未跟进 |

## 当前实现状态(截至第 50 回合,273 tests / clippy 0 / fmt clean)

全部 50 crates 已落地(ah-app / ah-contracts / ah-hub + 47 个 ah-plugins-*),
按职责分组如下(每项均为真实实现;完成度只以代码证据为准):

**内核 / 契约(3)**
- ah-hub:内核——注册表 / 事件 / Effect / Plugin / Profile,boot 即挂载、drop 即反注册;
- ah-contracts:全部 seam 契约与纯类型(llm/tools/fs/shell/code/sandbox/security/session/context/memory/retrieval/agent-loop/workflow/subagent/teams/evolving/rsi/telemetry/queue/mcp/transport/credentials/controller/operator/optimizer/trainer/runner/pregel/external/subagents 等);
- ah-app:profile 组装 + plugin_catalog(58 个目录项)+ boot + **ah-cli 交互入口**(任务输入、会话新建/切换/分叉)。

**模型 provider(3)**
- ah-plugins-openai:真实 LLM HTTP provider(SSE 流式 + 工具调用组装;key 先查 credentials seam 再 fallback 环境变量,两者皆无则挂载显式失败);
- ah-plugins-anthropic:真实 Messages API(system 顶层拆分 + tool_use/tool_result 块 + 流式);
- ah-plugins-mock:仅 llm boot 桩,只在 dev profile(prod 无 mock,CI mock 门禁强制)。

**核心 seam(27)**
- ah-plugins-agent-loop:真实 ReAct 循环(日志驱动,工具错误回喂模型);
- ah-plugins-tools:真实工具注册表(含工具执行管线 pre/post-execute waterfall);
- ah-plugins-sysop:真实受限文件系统与 shell 执行;
- ah-plugins-rails:ShellGuard 危险命令 / PathGuard 路径逃逸 / ToolBudget 调用上限 / 渐进披露 ApprovalRail;
- ah-plugins-session-log:会话事件日志(JSONL 落盘 + 投影 + create/fork/resume/checkpoint 多会话);
- ah-plugins-workflow:真实工作流引擎(Start/End/LLM/Tool/Loop/SubWorkflow/Parallel/Http/Intent/Questioner + 条件边 + 检查点续跑 + LLM 节点流式消费);
- ah-plugins-memory:真实持久化记忆 + remember/recall/forget 工具;
- ah-plugins-retrieval:真实知识库检索(BM25 分块 + 本地确定性向量 + ingest/search 工具,search 支持 bm25|vector 模式);
- ah-plugins-context:真实上下文引擎(token 预算组装 + 压缩/LLM 总结 + offload + reinject);
- ah-plugins-store:文件 / Redis / PostgreSQL(GaussDB 兼容)三后端,BaseKVStore/BaseMessageStore 同 seam 互换;
- ah-plugins-prompt:真实版本化 prompt 注册表({{var}} 渲染 + 缺失变量显式报错 + 文件持久化);
- ah-plugins-queue:文件 / Redis 后端消息队列(append-only JSONL/日志 + 消费游标,重启恢复);
- ah-plugins-workspace:真实工作区清单(workspace.json + 目标状态机,变更即落盘);
- ah-plugins-sandbox:真实策略化沙箱(sandbox.json 允许前缀/拒绝命令模式 + pre-execute rail);
- ah-plugins-code:真实代码执行(隔离 scratch + python3 子进程 + 超时强杀 + 输出/退出码);
- ah-plugins-web:真实 HTTP 客户端(ureq GET + 超时 + 状态/响应头/正文;TLS 需启用 ureq tls 特性);
- ah-plugins-security:真实安全检测(提示注入/敏感数据 guardrails + pre-execute rail);
- ah-plugins-subagent:真实子代理(隔离会话委派 + 预算 + 上下文注入 + delegate_task 工具);
- ah-plugins-subagents:真实类型化子代理(code/research/plan/verify + 工具白名单真实强制);
- ah-plugins-graph-memory:真实知识图谱记忆(实体抽取/共现关系/episode + JSONL 持久化 + 关键词检索/邻居遍历 + graph_* 工具);
- ah-plugins-pregel:真实超级步图引擎(状态通道/条件触发/中断/上限);
- ah-plugins-controller:真实控制器(任务 CRUD/状态机非法迁移拒绝/优先级/父子防环 + 执行器注册表 + 确定性意图识别);
- ah-plugins-runner:真实回调链(优先级降序 + retry/timeout/BREAK/ROLLBACK 逆序回滚 + 指标);
- ah-plugins-operator:真实自进化算子(LLM/tool/memory/skill 四参数句柄 + freeze + on_parameter_updated + 检查点);
- ah-plugins-optimizer:真实文本梯度优化器(评估问题 → 参数路由 → OperatorRegistry 应用,冻结/缺失显式记录);
- ah-plugins-trainer:真实自进化训练循环(基线评估 → 前向 → 梯度应用 → 验证门禁 → 改进推进 best + early stop);
- ah-plugins-rl:真实 RL 奖励函数(确定性线性:通过/失败/超时/工具错误/迭代项)。

**harness(5)**
- ah-plugins-cli:真实终端渲染(Claude Code 风格 ● Tool(args)/⎿ 摘要/☑☐ todo/⚙ 消息)+ 全 seam 子命令;
- ah-plugins-external:真实外部 CLI agent 运行时(流式长驻 stdin + 单发 argv,adapter 启动知识/完成标记/steer/abort);
- ah-plugins-autoharness:真实六阶段自动改进周期(assess→publish,真实 git 提交 + ci 门禁);
- ah-plugins-git:真实本地 git(init/status/add/commit/log/diff/branch,auto_harness 基建);
- ah-plugins-ci:真实 CI gate 运行器(子进程门禁 + 超时 + 通过/失败判定)。

**teams / evolving / rsi(3,第三梯队 A 已验收)**
- ah-plugins-teams:真实多 agent 团队(内存 + SQLite 持久化两运行时,任务板/依赖门控/成员校验/review/settle/run_task 真实委派 + swarmflow 编排 + 消息经 queue seam);
- ah-plugins-evolving:真实演进(轨迹从会话日志抽取 + 本地判据评估 + LLM judge 附加 + 优化建议 + experience JSONL 持久化);
- ah-plugins-rsi:真实 RSI(数据集确定性扩展 + LLM 生成、用例经 subagent 真实执行 + evolving 评估 + 结果分析 + 提示精化 + checkpoint 续跑 + single-harness 候选门禁)。

**extensions(5)**
- ah-plugins-mcp:真实 MCP 客户端(stdio 子进程 + streamable-HTTP 风格,JSON-RPC 2.0 + mcp_call_tool 工具);
- ah-plugins-transport:真实 A2A 风格传输(JSON-RPC 2.0 + SSE 流式 over HTTP,客户端 + 本地 HTTP 服务端);
- ah-plugins-credentials:真实凭据引用(环境变量 provider,openai.api_key → OPENAI_API_KEY 等映射可配置;get/list 真实读 env,set/remove 显式报错);
- ah-plugins-telemetry:真实 telemetry(内存 span + JSONL 导出 + OTLP/JSON 导出,agent/step 与 tools/post-execute 监听生成真实 span;semconv 留待后续);
- ah-plugins-oauth:真实设备码 OAuth(Device Authorization Grant 流 + pending/expired 映射)。

**dev_tools(4)**
- ah-plugins-agentbuilder:真实 agent 构建(NL→设计→DSL→WorkflowEngine 真实执行);
- ah-plugins-tune:真实训练流水线(subagent 执行 + evolving 评估/优化 + 最优跟踪);
- ah-plugins-skill:真实技能注册/持久化/评估(文件后端 + subagent 委派 + evolving 轨迹);
- ah-plugins-symphony:真实 symphony(能力注册/指纹/检索/计划/执行,JSONL 持久化 + 工具/subagent 真实执行)。

**工程状态**
- mock 门禁:prod profile 51 插件不含 ah-plugins-mock;dev 51 插件含 mock(llm boot 桩,真实 provider 需凭据时保留占位);CI 以 ah-app mock_gate 测试强制(prod 必须无 mock);
- 测试:273 tests 全过、clippy -D warnings 0、fmt clean(第 50 回合账目,见 REMAINING_PLAN.md);
- 工具执行管线(pre-execute/post-execute)已落地;
- 剩余:capability-map 标注 partial/留待后续 的项(browser/web/lsp 真实工具、远程沙箱、外部 provider、LLM 后端 guardrails、semconv、差分契约 Python 参考数据、跨平台验证等)。
- 本仓库禁止用文档声称完成度;完成度只以代码证据(测试 + 真实路径)为准。

## 文档维护纪律

1. 新文档在本文档集登记后生效;
2. 状态变更必须附实现路径与测试证据,禁止只改文档不改代码或反之;
3. 术语必须使用 glossary.md 定义;
4. 规划文档不在能力落地前创建(防止空转)。

