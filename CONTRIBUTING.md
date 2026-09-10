# 贡献指南

感谢参与 agent-harness。请先阅读根目录 [AGENTS.md](AGENTS.md)，再按 [文档索引](docs/README.md) 阅读 `architecture.md`、`hub-primer.md`、`capability-map.md`、`agent-guide.md` 和 `development.md`。本文约束贡献流程；架构和验收规则以这些文档为准。

## 项目边界

- 产品实现、默认测试和 CI 均为 Rust-only。
- Python 源码和 `differential/` 仅作为历史行为规格或可选审计资产，不是构建、运行时或默认验收依赖。
- 不提供 `openjiuwen.*` Python import 路径或 Python 对象模型兼容。

## 开发环境

需要 Rust stable、Cargo 和 Git。生产路径的开发或 E2E 可能还需要 Redis、Docker、外部 provider 及凭据。

本地配置必须放在 `.env` 或 `AH_ENV_FILE` 指定的 env 文件中；env 文件覆盖同名进程变量，找不到文件时应用应显式失败。真实密钥禁止提交，禁止在 issue、日志或补丁中输出。

开发 Profile 可直接运行：

```sh
cargo run --offline -p ah-app --bin ah-app -- profiles/dev.toml
```

## 开发原则

1. 先定义或确认 `ah-contracts` 中的 Seam、事件和纯类型契约，再实现 provider 与 consumer。
2. `ah-contracts` 不放业务实现、mock、unsupported 或 fallback。
3. 插件生产依赖只能使用 `ah-hub` 与 `ah-contracts`；插件之间不得通过具体类型耦合。
4. 所有注册返回可逆 `Effect`，后台任务、子进程和 socket 同样必须有明确生命周期。
5. mock 只能进入 `ah-plugins-mock` 或明确标注为测试的路径；生产失败必须显式报错，不得静默降级。
6. 适用时覆盖成功、非法输入、错误、取消、超时、恢复、序列化和持久化行为。
7. 文档中的当前状态、测试数字和 provider 结论必须来自当前 HEAD 的实际验证；implementation、production verification 和 Python parity 分开记录。

## 提交流程

1. 从当前分支创建主题分支，保持每次提交一个逻辑变更。
2. 修改前先查找现有契约、插件、测试和文档模式，避免引入第二套约定。
3. 代码与测试完成后更新受影响的能力地图、事件/配置目录或其他技术文档。
4. 提交前运行与改动相关的聚焦检查；共享契约或跨 crate 改动按需运行 workspace 门禁。
5. Pull request 描述变更内容、设计原因、验证命令及未验证项和外部依赖。

推荐提交前检查：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

覆盖率门禁：

```sh
cargo llvm-cov --workspace --fail-under-lines 80
```

不具备外部凭据或服务时，不要把 production E2E 写成通过；明确记录 `production unverified`。

## 提交信息

遵循 Conventional Commits：

```text
<type>(<scope>): <subject>

<body: WHAT + WHY + verification>
```

允许的常用 type：`feat`、`fix`、`refactor`、`docs`、`test`、`chore`、`perf`、`style`。例如：

```text
docs: clarify plugin contribution workflow
feat(memory): add persistent memory provider
```

## Pull Request 要求

- 说明用户可观察行为和涉及的 crate、seam 或 profile。
- 列出实际运行的命令及结果；不要复用历史回合的测试数字。
- 包含失败、取消、超时、恢复等适用边界的证据。
- 不提交 `.env`、凭据、生成日志或无关格式化变更。
- 维护者反馈应通过追加提交修正；合并前确保分支可干净应用。

## 报告问题

Issue 应包含最小复现、Rust/Cargo 版本、profile、实际命令和完整错误分类。请先确认问题不由缺失 env 文件、未启动外部服务或错误 Profile 导致；敏感值使用占位符。安全问题不要公开提交，按仓库维护者提供的私下渠道报告。

## 许可证

贡献内容按项目 [Apache License 2.0](LICENSE) 发布，并受 [NOTICE](NOTICE) 约束。
