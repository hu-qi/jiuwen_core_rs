# 剩余计划(REMAINING PLAN)

> 目标:按照能力地图完成剩余计划任务。当前 104+ 测试,18 crate。
> 执行模式:规划(本表)→ 派发执行对话 → 主对话验收(构建/测试/clippy/真实性/架构)。

## A. 第三梯队大域(按依赖)

| # | 任务 | 状态 | 验收要点 |
| --- | --- | --- | --- |
| 7 | teams(多 agent 任务协作) | 执行中 | 任务板/依赖/review/settle 真实状态迁移;run_task 真实委派 subagent |
| 8 | evolving(轨迹/evaluator/optimizer) | 待办 | 真实轨迹抽取、LLM-as-judge(本地判据优先)、优化管线 |
| 9 | rsi(数据生成/评测/优化编排) | 待办 | 端到端 RSI 流水线;真实 git/CI 基建 |

## B. 工程收尾

| 项 | 状态 | 说明 |
| --- | --- | --- |
| CI mock 门禁 | ✅ 0f55d52 | prod profile 无 mock 插件,测试+CI 强制 |
| 契约 fixtures(G-02) | 待办 | 每 seam 的成功/非法/超时/取消/恢复/序列化 golden fixtures |
| 覆盖率门禁(≥80%) | 待办 | llvm-cov 接入 CI |
| 差分契约(与 agent-core 对等) | 待办(大工程) | 语言中立 fixtures |
| 跨平台验证 | 待办 | Linux/Windows 目前未跑 |

## C. 域深化(capability-map 缺口)

| 域 | 任务 | 说明 |
| --- | --- | --- |
| core | context_engine / prompt / store(真实后端) / retrieval 向量化 / single_agent 深化 | 优先 context_engine 与 store 真实后端 |
| harness | 剩余 rails / browser/web/lsp 真实工具 / workspace-goal-manifest 细化 | rails 复用 pre-execute 挂载点 |
| extensions | store(Redis/GaussDB/ES)/ queue(Pulsar)/ sandbox(远程)/ openai OAuth / a2a / mcp-http | 真实协议路径 |
| teams 深化 | 外部 CLI 进程 / ZMQ / SQLite 持久化 | teams 基础落地后 |

## 执行顺序(当前)

7(teams)→ 验收 → 8(evolving)→ 验收 → 9(rsi)→ 验收;并行穿插 B 项;A/C 深化按依赖推进。
