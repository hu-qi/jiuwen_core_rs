# Python/Rust Differential(辅助行为审计)

Rust-only contract fixture 是主回归门禁;本目录的 Python/Rust differential 只在两端处于同一
抽象层且输入可确定时提供辅助行为审计。它不进入 Rust 构建、运行或生产 Profile,也不能替代
`ah-app/tests/rust_contract.rs` 的 Rust contract 验证。

## 布局

```text
fixtures/                      语言中立 fixture(仓库根)
  session.json                 Rust contract:append/derive/JSONL resume
  tools.json                   Rust contract:真实工具调用与错误分类
  controller.json              Rust contract:状态轨迹与确定性意图
  stop_condition.json          stop_condition Python/Rust differential
  llm_retry.json               LLMRetryRail detector/marker/backoff differential
  tool_retry.json              ToolCallResilienceRail exception differential
  task_completion.json         TaskCompletionRail prompt hook differential
  goal_manager.json             GoalManager 状态生命周期 differential
  prompt_attachment.json        PromptAttachment CRUD/filter/expiry differential
  runtime_model_switching.json TaskPlanningRail 运行时模型切换 differential
  team_inbox_format.json       ExternalTeamClient 入站消息/任务看板格式 differential
  team_inbox_fetch.json        ExternalTeamClient fetch mark-read/watch differential
references/                    Rust baseline(仅由 Rust 生成)
differential/                  可选 Python 辅助审计脚本
  run_python.py                可选 Python reference runner(驱动真实 openjiuwen)
  compare.py                   可选 Python outcome 与 Rust reference 对比
  run.sh                       可选辅助审计流程
  out/python/{seam}.json       Python 独立输出(不提交,运行生成)
```

## 运行

```sh
# Rust-only 主门禁(不需要 Python)
cargo test -p ah-app --test rust_contract

# 本地 Python 辅助审计(需要 Python 3.11+、pydantic 和 agent-core)
AGENT_CORE_ROOT=/path/to/agent-core \
PYTHON=/path/to/python3.11 \
bash differential/run.sh
# 仅运行 Rails、GoalManager、PromptAttachment、模型切换与 team inbox 差分
AGENT_CORE_ROOT=/path/to/agent-core \
bash differential/run.sh llm_retry tool_retry task_completion task_planning goal_manager prompt_attachment runtime_model_switching team_inbox_format team_inbox_fetch
```
CI 的 `rails-differential` job 使用固定的 agent-core commit,执行上述四个
LLM Rails seam;GoalManager、PromptAttachment 与 runtime model switching 可按需运行。
agent-core Python 没有与 Rust `after_model_call` callback-control 等价的 seam;
因此 cancellation callback 仅保留 Rust 行为测试,不伪造 Python differential。

Rust reference 回归仍可单独运行:

```sh
cargo test -p ah-app --test differential
AH_REFGEN=1 cargo test -p ah-app --test differential
```

## 纪律(与 docs/testing.md 一致)

1. Rust contract fixture 是 Rust-only 主门禁,不导入 Python。
2. `references/` 只能由 Rust 生成(`AH_REFGEN=1`);`run_python.py` 绝不写
   `references/`,只写 `differential/out/python/`。
3. Rust reference 是 regression baseline,不是独立 Python 参考结果。
4. `known_divergence` 只记录可同层比较后的真实语义差异;不可同层比较的行为标记为 `not-comparable`,不能通过削弱 contract 或 fixture 规避。
5. 比较时记录 agent-harness HEAD 与 agent-core HEAD,保证辅助审计可复现。

## 当前已覆盖 seam 与已知差异

| seam | case | 状态 |
| --- | --- | --- |
| stop_condition | max_rounds / token_budget / timeout / completion_promise 状态机 | 4/4 一致 |
| llm_retry | suffix detection / repeat-timeout markers / backoff | 10/10 一致 |
| tool_retry | exception type/marker retryability | 9/9 一致 |
| task_completion | Python TaskCompletionRail prompt hook | 3/3 一致 |
| task_planning | Python TaskPlanningRail prompt/model selection hook | 4/4 一致 |
| messager_inprocess | 单实例 pub-sub 盖章与退订 / p2p 注册发送退订 | 2/2 一致 |
| messager_inprocess | cross_instance_shared_bus | **known_divergence** |

### 已发现并记录的语义差异

- **总线作用域**:Python `_Bus` 是**进程全局**单例(所有 `InProcessMessager` 共享),
  Rust `InProcessMessager` 持有**每实例** `InProcessBus`。跨实例 fan-out(teammate 发布、
  leader 订阅)Python 可送达、Rust 收不到。Rust 需改为共享/全局总线才能对等。
- **sender_id 盖章条件**:Python 仅当消息对象**有 `sender_id` 字段且为空**时盖章
  (`hasattr` 判定);Rust 在字段**缺失或为空**时都盖章。fixture 当前用带 `sender_id`
  字段的消息规避该差异;字段缺失场景待 Rust 对齐后纳入。
- **多订阅者投递顺序**:Rust `InProcessBus` 用 `HashMap`,多订阅者时投递顺序非确定;
  Python 按订阅插入序。fixture 当前保持单订阅者。

### tools seam 的比较边界

`fixtures/tools.json` 是 Rust-only contract，不直接进入 Python runner。当前六类工具不能作为一个
同层 Python/Rust differential：Rust 提供的是 `tools` seam 下的同步 JSON provider，Python 的
`filesystem.py` 工具依赖 `SysOperation`、预读状态和 `ToolOutput`，`todo.py` 依赖 session-aware
异步文件系统，`cron.py` 依赖宿主注入的异步 backend，Python `memory.py` 是路径型 memory 文件
工具而 Rust `ah-plugins-memory` 是 key/tag 记忆 provider。强行比较会把参数名、生命周期和宿主
依赖差异误报成行为差异。

因此 P2-01 的证据边界为：

| capability | Python differential | Rust evidence |
| --- | --- | --- |
| edit / glob / grep | `not-comparable`：Python 需要 `SysOperation` 与不同输出模型 | `fixtures/tools.json`、sysop tests、production tool smoke |
| todo / cron | `not-comparable`：Python 使用不同的分工具 API 与 cron backend contract | common-tools tests、持久化/损坏存储 tests、Rust contract fixture |
| memory | `not-comparable`：两端 provider 抽象和持久化模型不同 | memory tests、key containment/reopen test、Rust contract fixture |

这不是 parity 通过声明；它是明确禁止把不可比较的 Python 结果伪装成 parity 证据。若未来需要严格
Python parity，必须先冻结双方共同的 provider/schema/生命周期 contract，再新增独立 fixture。

## 新增一个 seam

1. 在 `fixtures/` 加 `{seam}.json`(语言中立:只含输入与期望形状,不含实现);
2. `differential/run_python.py` 增加 `run_{seam}()` 驱动真实 openjiuwen 模块
   (隔离加载见文件头注释),输出与 Rust 侧**同一形状**;
3. `crates/ah-app/tests/differential.rs` 增加 `reference_{seam}` 测试
   (复用 `settle()` 与 `load_fixture()`);
4. `AH_REFGEN=1 cargo test -p ah-app --test differential` 生成 Rust reference;
5. `differential/compare.py` 的 `SEAMS` 加入 `{seam}`;
6. 端到端跑 `bash differential/run.sh`,把两端一致或标记 known_divergence 后提交。

## CI

`.github/workflows/ci.yml` 的 `rails-differential` job 对固定 agent-core revision 执行
`llm_retry`、`tool_retry`、`task_completion` 和 `task_planning`;任一未标记差异都会失败。
`stop_condition`、`messager_inprocess` 等历史 seam 仍是本地辅助审计,其中已登记的
`known_divergence` 不会阻断本地比较。输出必须记录双方 revision。
