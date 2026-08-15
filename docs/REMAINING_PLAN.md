# 剩余计划(REMAINING PLAN)

> 目标(已修正):**用 Rust 独立实现 agent-core 全部功能,不依赖 Python agent-core 运行时**
> (Python 源码仅在 /Volumes/coder/开源/rs_jiuwen/agent-core 作为规格参考;capability-map 为核对账本)。
> 当前 182+ 测试,31 crate。
>
> **第三梯队 A(teams / evolving / rsi)已全部完成**(df5584f)。
> **B-1 契约 fixtures(G-02)已落地**:fixtures/ 9 个 seam 的语言中立 golden + ah-app/tests/golden.rs 驱动真实实现验证。
> **B-2 覆盖率门禁已落地**:cargo llvm-cov 实测 workspace 行覆盖率 87.94%,CI 以 --fail-under-lines 80 强制。
>
> **账目更新**(第 43 回合,259 tests / clippy 0 / fmt clean):
> - LLM 流式 seam(ModelChunk/ToolCallDelta + ModelProvider::stream_chat 默认退化单块;OpenAI provider 真实 SSE 解析:data 行逐块,content/tool_calls delta 累加,[DONE] 结束;main.rs 流式演示)
>
> **账目更新**(第 42 回合,257 tests / clippy 0 / fmt clean):
> - Anthropic provider ah-plugins-anthropic(真实 Messages API:/v1/messages,system 顶层拆分,tool_use/tool_result 块,工具 schema→input_schema,x-api-key+anthropic-version 头;本地 HTTP 协议往返测试)
>
> **账目更新**(第 41 回合,252 tests / clippy 0 / fmt clean):
> - runner 回调链 ah-plugins-runner(优先级降序执行 + 链式传参;单回调 retry 上限/timeout 判错/BREAK 短路/ROLLBACK 与错误均逆序执行 rollback handler;CallbackMetrics 调用次数/耗时/错误率)
>
> **账目更新**(第 40 回合,248 tests / clippy 0 / fmt clean):
> - operator 自进化算子 ah-plugins-operator(LLM/tool/memory/skill 四参数句柄:tunables 冻结过滤/set_parameter freeze 检查/on_parameter_updated 回调/检查点 load_state;注册表按 id 取回)
>
> **账目更新**(第 39 回合,244 tests / clippy 0 / fmt clean):
> - controller 控制器 ah-plugins-controller(任务 CRUD/状态机非法迁移拒绝/优先级索引/父子层级防环;执行器注册表 + 同会话 working 冲突拒绝;确定性意图识别 create/pause/resume/cancel/switch 等)
>
> **账目更新**(第 38 回合,241 tests / clippy 0 / fmt clean):
> - PostgreSQL store 后端 ah-plugins-store-pg(真实 SQL:kv/messages 两表 + UPSERT + 增量读 + 幂等建表;GaussDB 兼容,与文件/Redis 同 seam 互换;docker-gated 测试真实连 ah-pg)
>
> **账目更新**(第 37 回合,239 tests / clippy 0 / fmt clean):
> - external CLI agent 运行时 ah-plugins-external(真实子进程:流式长驻 stdin + 单发每轮 argv 两风味;adapter 启动知识/完成标记/steer/abort/超时;TeamJoinDescriptor 编码;codex proto 辅助)
>
> **账目更新**(第 36 回合,236 tests / clippy 0 / fmt clean):
> - Redis 队列后端 ah-plugins-queue-redis(真实外部 provider:LIST 日志 + INCR 序号 + 游标持久,重启续消费;prod 换用,dev 保持本地)
>
> **账目更新**(第 35 回合,235 tests / clippy 0 / fmt clean):
> - cli 渲染(Claude Code 风格:● Tool(args)/⎿ 结果摘要/todo checkbox/⚙ 消息/推理默认隐藏;事件→块投影;ah-cli 实时渲染)
>
> **账目更新**(第 34 回合,230 tests / clippy 0 / fmt clean):
> - symphony 插件(能力注册/指纹/检索/计划/执行,JSONL 持久化 + 工具/subagent 真实执行;已接线 profiles + catalog)
>
> **账目更新**(第 33 回合,228 tests / clippy 0 / fmt clean):
> - agent_builder 790748c(NL→设计→DSL→工作流真实执行)
>
> **账目更新**(第 32 回合,225 tests):
> - 设备码 OAuth 7607726(真实 Device Authorization Grant 流)
>
> **账目更新**(第 31 回合,223 tests):
> - OTLP/JSON 导出 53afa6a(telemetry 缺口)
>
> **账目更新**(第 30 回合,222 tests):
> - 渐进披露 ApprovalRail 09bc486(tool-approval seam + pre-execute rail,批准集持久化)
>
> **账目更新**(第 29 回合,220 tests):
> - tune 训练流水线 409606a(真实 subagent 执行 + evolving 评估/优化 + 最优跟踪)
>
> **账目更新**(第 28 回合,218 tests):
> - graph/Pregel b89bbc6(超级步引擎 + 条件路由 + 中断 + 上限)
>
> **账目更新**(第 27 回合,215 tests):
> - A2A SSE 流式 a273190(text/event-stream 端点 + stream_send 客户端)
>
> **账目更新**(第 26 回合,214 tests):
> - skill 注册 + 评估 dbe36a2(文件后端 + subagent 委派 + evolving 轨迹)
>
> **账目更新**(第 25 回合,212 tests):
> - workflow 检查点续跑 d2bbbc1(JSONL 逐节点落盘 + resume)
>
> **账目更新**(第 24 回合,211 tests):
> - workflow Questioner 节点 af96eb5(经 queue 真实问答 + 超时)
>
> **账目更新**(第 23 回合,209 tests):
> - mcp-http 客户端 0293962(streamable-HTTP 风格 POST JSON-RPC)
>
> **账目更新**(第 22 回合,207 tests):
> - Redis 后端 543ee4b(真实外部 provider,SET/GET/DEL/KEYS,prod 换用)
>
> **账目更新**(第 21 回合,206 tests):
> - 类型化子代理 e824604(code/research/plan/verify + 工具白名单真实强制)
>
> **账目更新**(第 20 回合,204 tests):
> - agent_rl reward dac0c8d(确定性线性奖励函数)
>
> **账目更新**(第 19 回合,201 tests):
> - workflow Http + Intent 节点 6abc2c9(真实 HTTP / 关键字 + LLM 路由)
>
> **账目更新**(第 18 回合,198 tests):
> - evolving experience 持久化 bcf9d87(save/load/search JSONL + 跨重开恢复)
>
> **账目更新**(第 17 回合,196 tests):
> - auto-harness 六阶段编排 021af8d(assess→publish,真实 git 提交 + ci 门禁)
>
> **账目更新**(第 16 回合,193 tests):
> - evaluation_result_analyzer aa892da(确定性信号 + 根因归因 + analysis.json)
>
> **账目更新**(第 15 回合,190 tests):
> - git seam c25e510(真实子进程:init/add/commit/log/status/diff/branch)
> - ci seam ddd1157(子进程门禁 + 超时 + 通过判定)  —— auto_harness git/CI 基建完成
>
> **账目更新**(第 14 回合,182 tests;目标修正为不依赖 Python):
> - swarmflow 真实引擎 14644a5(agent_teams workflow 缺口,替换 MockWorkflowStep)
>
> 下一优先级:auto_harness 真实 git/CI → evaluation_result_analyzer → 外部 provider(docker)→
> evolving 持久化/agent_rl → core workflow/Pregel/controller → harness subagents/cli → dev_tools。
>
> **账目更新**(第 13 回合,173 tests):
> - rsi 多轮编排 5c25592(run_rounds:评测→精化→checkpoint 续跑)
> - web_fetch / run_code 真实工具 f636780(agent 可调用,seam 消费方闭环)
>
> **账目更新**(第 12 回合,169 tests):
> - transport(A2A 风格 JSON-RPC over HTTP)35e0799 —— capability-map **零 missing** 达成
>
> **账目更新**(第 11 回合,166 tests):
> - ah-cli 全 seam 子命令 fac02b3(teams/rsi/workspace/web/queue/code)+ 真实二进制 e2e 冒烟测试
>
> **账目更新**(第 10 回合,165 tests):
> - code 真实执行 cb1f07a(隔离 scratch + python3 子进程 + 超时强杀)
> - web 真实 HTTP 3ed25db(ureq GET + 超时,本地 TCP 真实协议测试)
>
> **账目更新**(第 9 回合,159 tests):
> - teams 消息经 queue c6f929a(send_message/messages,queue seam 消费方闭环)
> - sandbox 策略化沙箱 50bd531(sandbox.json 策略 + pre-execute rail)
>
> **账目更新**(第 8 回合,154 tests):
> - queue 本地基础 1840fc3(日志+游标 offset 语义,重启恢复)
> - workspace 清单与目标 7cfa80f(workspace.json + 目标状态机)
>
> **账目更新**(第 7 回合,150 tests):
> - teams SQLite 持久化 be31244(SqliteTeamRuntime 真实事务,重启恢复)
> - session 检查点 2cea108(checkpoint/restore 命名快照,seq 续接)
>
> **账目更新**(第 6 回合,145 tests):
> - retrieval 向量化 9483740(哈希 n-gram TF embedding + 余弦,CJK 二元组,search 工具 vector 模式)
> - harness rails ebc089c(PathGuard 路径逃逸 + ToolBudget 调用上限,复用 pre-execute 挂载点)
> - prompt seam 三角闭环 6aa8d54(agent-loop 消费方:注册模板渲染注入系统提示)
>
> **账目更新**(第 5 回合,137 tests):
> - context 引擎 8c59fe4(token 预算组装/压缩/offload/reinject)
> - store 本地文件后端 c039873(BaseKVStore + BaseMessageStore)
> - prompt 版本化注册表 f93810e({{var}} 渲染 + 缺失显式报错 + 文件持久化)
> - context seam 三角闭环 1ed1cb8(agent-loop 与 subagent 作为消费方按预算组装)
>
> **历史账目**:
> - teams 95abe79 · evolving a089fd9 · rsi df5584f(第三梯队 A 全部交付验收)
> - G-02 契约 fixtures 7a7d7de(fixtures/ + ah-app/tests/golden.rs)
> - 覆盖率门禁 51da82b(CI coverage job + 实测 87.94%)
> - 子代理日志补齐 AgentStep(随 df5584f)
>
> **下一步建议**:差分契约与跨平台需外部环境,建议委派任务书;域深化按 C 节推进
> (context_engine 优先,其次 store 真实后端 / retrieval 向量化)。
> 执行模式:规划(本表)→ 派发执行对话 → 主对话验收(构建/测试/clippy/真实性/架构)。

## A. 第三梯队大域(按依赖)

| # | 任务 | 状态 | 验收要点 |
| --- | --- | --- | --- |
| 7 | teams(多 agent 任务协作) | ✅ 已验收 | 任务板/依赖/review/settle 真实状态迁移;run_task 真实委派 subagent;成员校验;teams/task 事件 |
| 8 | evolving(轨迹/evaluator/optimizer) | ✅ 已验收 | 真实轨迹抽取(会话日志配对/预算/完成标志);本地判据评估 + LLM judge 附加(不可用显式记录);优化建议真实规则推导 + LLM 附加 |
| 9 | rsi(数据生成/评测/优化编排) | ✅ 已验收 | 端到端 RSI:数据集确定性扩展、用例经 subagent 真实执行 + evolving 评估、报告聚合、提示精化、JSONL checkpoint 续跑 |

## B. 工程收尾

| 项 | 状态 | 说明 |
| --- | --- | --- |
| CI mock 门禁 | ✅ 0f55d52 | prod profile 无 mock 插件,测试+CI 强制 |
| 契约 fixtures(G-02) | ✅ 已落地 | fixtures/ 9 seam golden(fs/session/tools/memory/retrieval/security/evolving/teams/rsi),成功/非法/序列化/恢复类目;超时/取消多为 N/A(本地无超时语义),后续 seam 补充 |
| 覆盖率门禁(≥80%) | ✅ 已落地 | llvm-cov 实测 87.94%;CI coverage job --fail-under-lines 80 |
| 差分契约(与 agent-core 对等) | 机制 + Rust 基线已落地(本回合 references/ 5 seam);Python 参考数据待外部生成 | references/ 完整输出快照 + differential.rs 断言门禁 |
| 跨平台验证 | 待办 | Linux/Windows 目前未跑 |

## C. 域深化(capability-map 缺口)

| 域 | 任务 | 说明 |
| --- | --- | --- |
| core | context_engine ✅ / store 本地文件后端 ✅ / prompt ✅(+agent-loop 消费)/ retrieval 向量化 ✅ / harness rails ✅(本回合)| 下一步:browser/web/lsp 真实工具(需外部)或 teams 深化 |
| harness | 剩余 rails ✅ / workspace-goal-manifest ✅(本回合)/ browser/web/lsp 真实工具 | workspace 清单与目标已落地;browser/web/lsp 需外部进程或网络,留待后续 |
| extensions | store(Redis/GaussDB/ES)/ queue(Pulsar)/ sandbox(远程)/ openai OAuth / a2a / mcp-http | 本地 queue ✅、本地 sandbox 策略 ✅(本回合);外部协议路径需容器/网络,留待后续 |
| teams 深化 | 外部 CLI 进程 / ZMQ / SQLite 持久化 / 消息 | SQLite 持久化 ✅;消息经 queue seam ✅(本回合);外部 CLI 进程 / ZMQ 留待后续 |

## 执行顺序(当前)

7(teams)→ 验收 → 8(evolving)→ 验收 → 9(rsi)→ 验收;并行穿插 B 项;A/C 深化按依赖推进。
