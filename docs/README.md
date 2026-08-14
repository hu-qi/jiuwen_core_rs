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

以下文档描述尚未实现的子系统。为避免"文档先于代码"的空转,只在对应能力
达到 partial 后创建,并在本表登记:

| 规划文档 | 触发条件 | 内容 |
| --- | --- | --- |
| tool-catalog.md | tools seam 落地 | 工具注册表、工具分类 |
| tool-execution-pipeline.md | 工具执行管线 | 执行管线、鉴权、超时、回滚 |
| agent-lifecycle.md | agent-loop 落地 | turn/step 生命周期、事件序列 |
| persistence-catalog.md | session log 落地 | 持久化格式与版本策略(**已创建**) |
| cookbook.md | 插件数 > 3 | 面向插件作者的扩展配方集 |

## 当前实现状态(截至框架提交)

- 已实现:ah-hub 内核;ah-contracts(llm/tools/fs/shell 四个 seam + agent-loop 服务);
  ah-plugins-tools(真实工具注册表);ah-plugins-sysop(真实受限文件系统与 shell 执行);
  ah-plugins-openai(真实 LLM HTTP provider);ah-plugins-agent-loop(真实 ReAct 循环);
  ah-plugins-rails(真实 rails 挂在 tools/pre-execute waterfall 上:ShellGuard 危险命令 / PathGuard 路径逃逸 / ToolBudget 调用上限);
  ah-plugins-session-log(真实会话事件日志:JSONL 落盘 + 投影 + 多会话管理 create/fork/resume);
  ah-plugins-workflow(真实工作流引擎:Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + 条件边);
  ah-plugins-memory(真实持久化记忆 + remember/recall/forget 工具);
  ah-plugins-retrieval(真实知识库检索:BM25 分块 + 本地确定性向量(哈希 n-gram TF + 余弦) + ingest_knowledge/search_knowledge 工具,search 支持 bm25|vector 模式);
  ah-plugins-security(真实安全检测:提示注入/敏感数据 guardrails + tools/pre-execute rail);
  ah-plugins-subagent(真实子代理:隔离会话委派 + 预算 + 上下文注入 + delegate_task 工具);
  ah-plugins-teams(真实多 agent 团队:内存 + SQLite 持久化两套运行时,任务板 + 依赖门控 + 成员校验 + review 票 + settle 多数决 + run_task 真实委派 subagent);
  ah-plugins-evolving(真实演进:轨迹从会话日志抽取 + 本地判据评估 + LLM judge 附加 + 优化建议);
  ah-plugins-rsi(真实 RSI:数据集生成 + 用例经 subagent 真实执行 + evolving 评估 + 提示精化 + JSONL checkpoint 续跑);
  ah-plugins-context(真实上下文引擎:token 预算组装 + 摘录压缩/LLM 总结 + offload JSONL + 摘要 reinject);
  ah-plugins-store(真实文件后端 store:BaseKVStore JSON 文件 + BaseMessageStore JSONL channel);
  ah-plugins-prompt(真实版本化 prompt 注册表:{{var}} 渲染 + 缺失变量显式报错 + 文件持久化);
  ah-plugins-queue(真实文件后端消息队列:每 channel append-only JSONL + 消费游标,重启恢复);
  ah-plugins-workspace(真实工作区清单:workspace.json + 目标状态机,变更即落盘);
  ah-plugins-mcp(真实 MCP stdio transport:子进程 + JSON-RPC 2.0 + mcp_call_tool 工具);
  ah-plugins-credentials(真实凭据引用:环境变量 provider,openai.api_key → OPENAI_API_KEY 等映射可配置;get/list 真实读 env,set/remove 显式报错);
  ah-plugins-telemetry(真实 telemetry:内存 span 记录 + JSONL 导出,agent/step 与 tools/post-execute 监听生成真实 span;OTLP 导出留待后续);
  工具执行管线(pre-execute/post-execute)已落地;
  ah-app(demo 端到端 + **ah-cli 交互入口**:任务输入、会话新建/切换/分叉)。
- 仅剩一个 boot 桩:ah-plugins-mock(llm,真实 provider 需凭据时仍保留占位)。
- 规划:其余 seam 与插件(见 capability-map.md)。
- 本仓库禁止用文档声称完成度;完成度只以代码证据(测试 + 真实路径)为准。

## 文档维护纪律

1. 新文档在本文档集登记后生效;
2. 状态变更必须附实现路径与测试证据,禁止只改文档不改代码或反之;
3. 术语必须使用 glossary.md 定义;
4. 规划文档不在能力落地前创建(防止空转)。

