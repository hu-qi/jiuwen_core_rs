# 测试与对等验证

本项目的测试必须回答四个不同问题:Rust 实现是否正确、契约是否稳定、生产组合是否可运行、
以及行为是否与 agent-core(Python)一致。四者不能互相替代。

## 测试分层

| 层 | 证明什么 | 位置/命令 | 当前状态 |
| --- | --- | --- | --- |
| 单元测试 | Rust 内核和插件内部逻辑 | 各 crate `#[cfg(test)]`;`cargo test -p <crate>` | 已广泛覆盖;阶段一新增 10 个插件共 117 个单元测试 case 通过 |
| Golden fixture | Rust seam 对固定语言中立样例的契约稳定性 | `fixtures/` + `ah-app/tests/golden.rs` | 已有 9 个 seam |
| Rust contract fixture | Rust-only 规范化输入、状态轨迹、错误分类和恢复结果 | `fixtures/` + `ah-app/tests/rust_contract.rs` | 已接入 session、tools、controller、agent-loop、workflow、application;fixture schema version=1 |
| Rust regression reference | Rust 当前完整可观测输出不发生非预期变化 | `references/` + `ah-app/tests/differential.rs` | 已覆盖 session、security、retrieval、teams、evolving、messager 以及 application/controller traces |
| Rust-only contract | Rust 输入、状态、错误、恢复和序列化行为符合已登记规格 | `fixtures/` + `ah-app/tests/rust_contract.rs` | 主验收门禁;不加载 Python |
| Historical Python audit | 与历史 agent-core 行为的辅助比较 | `differential/` | 非 CI、非产品依赖;没有同层 Rust 实现时不得强行比较 |
| Production composition | prod profile 可解析、依赖闭合且无 mock | profile/catalog/依赖图测试 | mock exclusion/static composition 已有;10 个新增插件已进入 workspace、catalog 和 dev/prod Profile |
| Production boot/E2E | 无 mock 的真实组合可启动并执行 | 本地 HTTP fixture、服务容器或真实凭据 | dev Profile boot + demo E2E 已验证;prod 需要真实 Redis/OpenAI 等依赖 |
| 覆盖率 | Rust 测试执行到的代码比例 | `cargo llvm-cov --workspace --fail-under-lines 80` | CI 有门禁;当前 HEAD 数字须以完整实测为准,不得引用历史数字 |

## Golden fixture

Golden fixture 用于验证一个 Rust seam 的固定输入输出。建议至少覆盖:

1. 成功路径;
2. 非法输入和显式错误;
3. 超时;
4. 取消;
5. 恢复;
6. 序列化和版本兼容。

Golden fixture 不运行 Python,因此不能单独证明 Python parity。

## Rust contract fixture

`ah-app/tests/rust_contract.rs` 是 Rust-only 的 fixture runner。它读取
`fixtures/{session,tools,controller,agent_loop,workflow,application}.json`,要求 `schema_version=1` 和文件名/seam 一致,
再执行真实 Rust seam 并比较规范化 outcome。fixture 不记录时间、随机 ID 或实现内部结构,
只记录可观察的状态、顺序、错误分类和恢复结果。

该 runner 是 Rust contract 的主回归门禁;它不导入 Python,也不生成 Python reference。

Session 恢复专项使用 `cargo test -p ah-plugins-session-log --lib` 与
`cargo test -p ah-plugins-agent-loop --lib` 验证:

- 崩溃留下未完成 JSONL 末行时只截断最后一条损坏记录,完整损坏行仍显式报错;
- checkpoint/fork/restore 通过临时文件、同步和原子替换避免半快照;
- `cross_process_appenders_keep_unique_contiguous_sequences` 启动多个真实子进程,
  验证追加后的 seq 唯一、连续且 JSONL 可重开;
- `recovers_pending_tool_call_after_restart` 验证 Assistant tool call 已落盘而
  ToolResult 缺失时,重启后的 agent-loop 先补执行工具再请求模型。

- `cross_process_claim_has_one_owner` 启动多个真实子进程竞争同一工具调用,
  验证 claim 事件和 ToolResult 各只出现一次。
- `reconstructs_complete_stream_delta_after_restart` 验证崩溃前已记录的完整
  stream tool-call delta 会在重启恢复扫描中重建为待执行调用。
- `persists_partial_stream_tool_call_before_timeout` 验证模型流式 delta、工具调用
  delta 和超时前的部分工具调用均已落盘,而非只在内存中保留。
- 工具结果 payload 具有 `status=completed/error/unknown`;unknown 投影会显式告诉模型
  需要人工核对,避免把未知执行状态当成成功结果。

恢复工具调用默认失败关闭:仅声明 `idempotent() = true` 且通过 `invoke_with_id` 消费稳定
call ID 的工具允许自动补执行;非幂等或未声明工具写入 `ToolResult(status="unknown")`,返回
`AgentFailure::ToolRecoveryRequired`,并验证后续恢复不会重复执行。
## Rust regression reference

`ah-app/tests/differential.rs` 读取 `references/{seam}.json`,比较 Rust 实现的完整可观测
输出。设置 `AH_REFGEN=1` 时,reference 由 Rust 自己重写。

reference 防止 Rust 行为意外变化,但不构成外部实现的对等证明。提交 reference 变化时必须
解释行为变化,禁止仅为通过测试而重生成。

## Historical Python audit

`differential/run_python.py` 和 `differential/compare.py` 不是 Rust 产品代码,不参与默认 CI、
Rust 构建或生产运行。它们仅保留给需要审计历史 agent-core 行为的开发者；当两边不处于同一
抽象层时,结果必须记录为 `not-comparable`,不能用复制 Rust 逻辑的方式制造 Python outcome。

Rust-only 验收流程:

1. 定义版本化 Rust fixture 和规范化输出 schema;
2. 用 Rust contract runner 执行真实 Rust seam;
3. 用 Rust regression reference 固化完整可观测输出;
4. 对成功、失败、取消、超时、恢复和持久化路径执行 Rust 测试;
5. 通过 production smoke 和适用的真实协议 E2E。

## Production 验证

生产验证分三层:

1. **Mock exclusion**:prod profile 不包含 `ah-plugins-mock`;当前已有测试;
2. **Static composition**:所有插件名可从 catalog 解析,provides/inject 依赖闭合且无重复/环;已由 `ah-app/tests/static_composition.rs` 覆盖(dev/prod 双 profile,镜像 `mount_all` 语义、不触发 apply;曾抓出 prod 缺 `ah-plugins-model-backup` 的组合 bug);
3. **Boot smoke/E2E**:无 mock 完成 `boot()`、Application invoke、Controller 调度/恢复和 Workflow stream;`production-p1` CI job 已用 Redis service + 本地确定性 HTTP model fixture 执行。

真实 provider E2E 仍可使用本地 HTTP fixture、服务容器或真实凭据。因缺凭据跳过时必须输出明确原因,
不得以 mock 替代后仍标记 production verified。

## 覆盖率

- CI 要求 workspace 行覆盖率不低于 80%;
- 新改动域应维持不低于 80%;
- 历史文档中的覆盖率只代表历史 commit;
- 当前覆盖率必须引用 CI run 或当前 HEAD 的完整 `cargo llvm-cov` 结果。

高覆盖率证明测试执行范围,不证明 Python parity 或 production E2E。
## Definition of Done 测试要求

能力标记为 Rust implementation `done` 必须同时满足:

1. 生产实现可调用,无 mock、静默 fallback 或未注入替身;
2. 单元/Golden/contract 覆盖成功和失败语义;
3. 相关 production E2E 通过,或明确限定该能力不需要外部系统;
4. 覆盖率和现有 CI 门禁通过;
5. Python differential 若可同层比较则单独记录;不可比较时明确记录原因。

能力标记为严格 Python parity `done` 还必须额外满足:

1. Python/Rust differential 通过;
2. 证据记录实现路径、测试名和双方 commit。

在 Python differential 建立前,只能将能力标记为 implementation done 或 parity partial/unverified,
不得笼统宣称与 agent-core 完全对等。
