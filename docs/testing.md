# 测试与对等验证(testing.md)

> 本项目的目标是完整实现 agent-core(Python)全部功能。测试策略围绕一个核心问题:
> **如何证明 Rust 行为与 Python 对等**。

## 1. 测试分层

| 层 | 内容 | 位置/命令 | 必过门禁
| --- | --- | --- | --- |
| 单元 | 内核机制、插件内部逻辑 | 各 crate 内 #[cfg(test)] | cargo test --workspace
| 契约 | 每个 seam 的 golden fixtures | fixtures/ + ah-app/tests/golden.rs(已落地:9 seam) | cargo test --workspace
| 差分 | 与 agent-core 行为对等 | 语言中立 fixtures(规划) | cargo test --workspace
| e2e | 真实 provider/传输/子进程 | integration 标记;缺凭据自动跳过 | CI 存在性检查

## 2. 契约 fixtures(golden fixtures)

每个 seam 至少覆盖六类样例:

1. 成功路径(正常输入 → 期望输出);
2. 非法输入(错误、拒绝、显式报错);
3. 超时(超时语义与错误);
4. 取消(取消传播、状态一致性);
5. 恢复(断点续跑、崩溃恢复);
6. 序列化(往返一致、版本兼容)。

fixtures 使用语言中立格式(JSON/YAML),不 import Python 代码。

## 3. 差分契约(与 agent-core 对等)

方法(核心纪律):

- 对每个 Python 能力,从 agent-core 的公开行为面(输入/输出/状态迁移/错误/持久化/协议)
  提炼**语言中立契约**;
- 同一组 fixtures 分别喂给 Python(参考运行)与 Rust(被测),比较结果;
- Python 参考运行**不在本仓库内**:本仓库零 Python 源码,fixtures 由外部生成并固化;
- 差分契约通过 = 行为对等的证据;单靠本地/mock 测试通过 ≠ 完成。

## 4. 覆盖率门禁

- workspace 行覆盖率 ≥ 80%,新改动域不得低于 80%;
- CI 用 cargo llvm-cov(或等价工具)统一测量,不以本地手动数字为准;
- 未覆盖路径必须解释(unsupported 分支、平台差异、外部协议不可测部分)。

## 5. 凭据与外部依赖策略

- 真实 provider e2e 需要凭据(如 OPENAI_API_KEY);未设置时自动跳过并标记;
- 跳过必须有理由,不允许用 mock 冒充真实 e2e;
- 容器化依赖(Redis/Pulsar/ES 等)在 CI 中用服务容器或测试容器;本地可选。

## 6. 与 CI 门禁的关系

见 development.md §4。测试门禁要点:

1. cargo fmt --all --check;
2. cargo clippy --workspace --all-targets -- -D warnings;
3. cargo test --workspace;
4. cargo test --workspace --all-features;
5. mock 门禁(生产 profile 无 mock 插件);
6. 覆盖率 ≥ 80%。

## 7. Definition of Done 中的测试要求

能力达到 done 必须同时满足(development.md §6):

1. 生产路径真实执行,有测试证据(file:line);
2. 差分契约或 golden fixture 通过;
3. 相关 e2e 通过或有明确跳过理由;
4. 覆盖率达到门禁。
