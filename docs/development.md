# 开发流程(development.md)

> 所有贡献者必读。提交代码前请确保通过本文定义的检查。

## 1. 前置条件与搭建

- Rust stable toolchain(CI 与本地保持一致);
- git 2.26+;
- 可选:真实 provider 的凭据(OpenAI 等),仅真实 e2e 需要,缺失时自动跳过。

```sh
cargo build --workspace
cargo test --workspace
cargo run -p ah-app        # 从 profiles/dev.toml 启动(全 mock)
```

## 2. 工作包生命周期

每个能力(见 capability-map.md)按四阶段推进,每阶段都有独立验收:

| 阶段 | 产出 | 验收 |
| --- | --- | --- |
| 1. 契约 | ah-contracts 中的 seam trait + 纯类型 + 契约单测 | cargo test -p ah-contracts;契约零实现检查 |
| 2. Mock | ah-plugins-mock 中的确定性实现 + 插件注册 | 系统可 boot;dev profile 端到端可运行 |
| 3. 真实 | ah-plugins-* 生产实现 | 真实协议/持久化/子进程路径 + 集成测试;profile 从 mock 切换到真实 |
| 4. 对等 | 差分契约 + golden fixtures + e2e | 与 agent-core(Python)行为对等;验收证据 file:line |

**阶段推进规则**:

- 阶段 3/4 之间允许并行(不同能力);
- 任何阶段都不得在契约层引入实现;
- unsupported/todo!/unimplemented! 不允许出现在生产路径——要么实现,
  要么显式返回错误并登记为 missing。

## 3. 测试策略

### 3.1 测试分层

| 层 | 内容 | 命令 |
| --- | --- | --- |
| 单元 | 内核机制、插件内部逻辑 | cargo test -p <crate> |
| 契约 | 每个 seam 的 golden fixtures(成功/非法输入/超时/取消/恢复/序列化) | cargo test -p ah-contracts --test fixtures(规划) |
| 差分 | 与 agent-core 行为对等(语言中立 fixtures,不 import Python) | 每 seam 一个契约套件(规划) |
| e2e | 真实 provider/传输/子进程;缺凭据自动跳过 | cargo test --workspace 中的 integration 标记 |

### 3.2 覆盖要求

- workspace 行覆盖率 >= 80%,新改动域不得低于 80%;
- CI 以 cargo llvm-cov(或等价工具)门禁为准,不以本地手动测量为准。

## 4. CI 门禁(.github/workflows/ci.yml)

1. cargo fmt --all --check;
2. cargo clippy --workspace --all-targets -- -D warnings;
3. cargo test --workspace;
4. cargo test --workspace --all-features(所有 feature 必须可编译可测);
5. **mock 门禁**:展开生产 profile,若包含 ah-plugins-mock 插件则失败;
   同时拒绝:未批准的 fallback、空 feature、未登记的 Python/外部适配器、覆盖率低于 80%;
6. 文档门禁:docs 中引用的路径存在、能力状态表与代码证据一致(规划)。

## 5. 提交规范(Conventional Commits)

```text
<type>(<scope>): <subject>

<body: 说明 WHAT 和 WHY>
```

- type:feat / fix / refactor / docs / test / chore / perf / style;
- scope:crate 或域,如 ah-hub、ah-contracts、plugins-llm、docs;
- subject 祈使句、小写开头、<=72 字符、无句号;
- 一次提交一个逻辑变更;不要混入无关的 fmt/重构。

示例:feat(ah-hub): support parallel dispatch in EventBus

## 6. 完成定义(Definition of Done)

一个能力达到 done 必须同时满足:

1. 契约层有 trait/类型,且零实现;
2. 生产插件真实执行路径(非 mock/fallback/unsupported),有测试证据(file:line);
3. 差分契约或 golden fixture 通过,证明与 agent-core 行为对等;
4. 相关 e2e(真实 provider/协议)通过或有明确跳过理由;
5. fmt / clippy / 覆盖率达到门禁;
6. capability-map.md 状态更新为 done,并附实现路径与测试证据。

## 7. 变更纪律

- **插件,不是改循环**:新行为挂在文档化的扩展点;改 agent 循环/内核需要更新 architecture.md;
- **注册即 effect**:任何注册返回 Effect,无 guard 的裸注册视为缺陷;
- **证据优先**:状态变更必须带测试与实现证据,禁止仅凭描述推进;
- **不静默降级**:真实路径失败必须显式报错,不得切到本地 mock 继续跑。

