# 测试与对等验证

本项目的测试必须回答四个不同问题:Rust 实现是否正确、契约是否稳定、生产组合是否可运行、
以及行为是否与 agent-core(Python)一致。四者不能互相替代。

## 测试分层

| 层 | 证明什么 | 位置/命令 | 当前状态 |
| --- | --- | --- | --- |
| 单元测试 | Rust 内核和插件内部逻辑 | 各 crate `#[cfg(test)]`;`cargo test -p <crate>` | 已广泛覆盖 |
| Golden fixture | Rust seam 对固定语言中立样例的契约稳定性 | `fixtures/` + `ah-app/tests/golden.rs` | 已有 9 个 seam |
| Rust regression reference | Rust 当前完整可观测输出不发生非预期变化 | `references/` + `ah-app/tests/differential.rs` | 已有 5 个 seam |
| Python/Rust differential | 同一输入下 Python 与 Rust 的公开行为一致 | 外部 Python runner + 语言中立 fixture + Rust runner | partial(基础设施已落地:`differential/` + CI job;stop_condition、messager_inprocess 两 seam 已验证一致,1 个已知差异已记录;首批六 seam 待接入) |
| Production composition | prod profile 可解析、依赖闭合且无 mock | profile/catalog/依赖图测试 | 仅 mock exclusion 已验证 |
| Production boot/E2E | 无 mock 的真实组合可启动并执行 | 本地协议 fixture、服务容器或真实凭据 | partial |
| 覆盖率 | Rust 测试执行到的代码比例 | `cargo llvm-cov --workspace --fail-under-lines 80` | CI 有门禁;当前 HEAD 数字须以 CI 实测为准 |

## Golden fixture

Golden fixture 用于验证一个 Rust seam 的固定输入输出。建议至少覆盖:

1. 成功路径;
2. 非法输入和显式错误;
3. 超时;
4. 取消;
5. 恢复;
6. 序列化和版本兼容。

Golden fixture 不运行 Python,因此不能单独证明 Python parity。

## Rust regression reference

`ah-app/tests/differential.rs` 当前读取 `references/{seam}.json`,比较 Rust 实现的完整可观测
输出。设置 `AH_REFGEN=1` 时,reference 由 Rust 自己重写。

因此当前 reference 的准确名称是 **Rust regression reference**。它可以防止 Rust 行为意外变化,
但不是独立的 Python 参考结果。提交 reference 变化时必须解释行为变化,禁止仅为通过测试而重生成。

## Python/Rust differential

严格对等需要独立参考运行:

1. 定义语言中立 fixture 和规范化输出 schema;
2. 在 agent-core 环境运行 Python reference runner;
3. 在 agent-harness 运行 Rust runner;
4. 比较输出、错误类型、状态迁移、事件顺序、持久化、取消、超时和恢复;
5. 将差异作为 CI 失败,并记录两边 commit。

首批范围为 application、agent-loop、session、controller、workflow、tools。Python runner 必须独立于
Rust reference 生成流程;`AH_REFGEN=1` 不能生成或覆盖 Python reference。

## Production 验证

生产验证分三层:

1. **Mock exclusion**:prod profile 不包含 `ah-plugins-mock`;当前已有测试;
2. **Static composition**:所有插件名可从 catalog 解析,provides/inject 依赖闭合且无重复/环;已由 `ah-app/tests/static_composition.rs` 覆盖(dev/prod 双 profile,镜像 `mount_all` 语义、不触发 apply;曾抓出 prod 缺 `ah-plugins-model-backup` 的组合 bug);
3. **Boot smoke/E2E**:无 mock 完成 `boot()` 和一次 `ApplicationRuntime::invoke`;当前缺统一门禁。

真实 provider E2E 可使用本地 HTTP fixture、服务容器或真实凭据。因缺凭据跳过时必须输出明确原因,
不得以 mock 替代后仍标记 production verified。

## 覆盖率

- CI 要求 workspace 行覆盖率不低于 80%;
- 新改动域应维持不低于 80%;
- 历史文档中的覆盖率只代表历史 commit;
- 当前覆盖率必须引用 CI run 或当前 HEAD 的完整 `cargo llvm-cov` 结果。

高覆盖率证明测试执行范围,不证明 Python parity 或 production E2E。

## Definition of Done 测试要求

能力标记为严格 parity `done` 必须同时满足:

1. 生产实现可调用,无 mock、静默 fallback 或未注入替身;
2. 单元/Golden 覆盖成功和失败语义;
3. Python/Rust differential 通过;
4. 相关 production E2E 通过,或明确限定该能力不需要外部系统;
5. 覆盖率和现有 CI 门禁通过;
6. 证据记录实现路径、测试名和双方 commit。

在 Python differential 建立前,只能将能力标记为 implementation done 或 parity partial/unverified,
不得笼统宣称与 agent-core 完全对等。
