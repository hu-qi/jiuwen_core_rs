# 剩余计划(REMAINING PLAN)

> 目标(已修正):**用 Rust 独立实现 agent-core 全部功能,不依赖 Python agent-core 运行时**
> (Python 源码仅在 /Volumes/coder/开源/rs_jiuwen/agent-core 作为规格参考;capability-map 为核对账本)。
> 当前 182+ 测试,31 crate。
>
> **第三梯队 A(teams / evolving / rsi)已全部完成**(df5584f)。
> **B-1 契约 fixtures(G-02)已落地**:fixtures/ 9 个 seam 的语言中立 golden + ah-app/tests/golden.rs 驱动真实实现验证。
> **B-2 覆盖率门禁已落地**:cargo llvm-cov 实测 workspace 行覆盖率 87.94%,CI 以 --fail-under-lines 80 强制。
>
> **账目更新**(第 15 回合,190 tests / clippy 0 / fmt clean):
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
