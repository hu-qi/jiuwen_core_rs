# agent-harness

agent-harness 是一个使用 Rust 构建的插件化 agent harness，目标是覆盖 openJiuwen agent-core 的公开行为，并以低耦合的契约、事件、服务注册和 Profile 组合能力支持可扩展的 agent 应用。

项目完全以 Rust 实现和运行。agent-core 的 Python 源码仅作为历史行为规格参考，不是构建、测试或运行时依赖；本项目不提供 `openjiuwen.*` Python import 路径或 Python 对象模型兼容。

## 能力概览

- `ah-contracts`：Seam、事件和跨插件纯类型契约。
- `ah-hub`：ServiceRegistry、EventBus、Plugin、Effect 和 Profile 组合内核。
- `ah-plugins-*`：模型、工具、工作流、agent loop、会话、记忆、团队、遥测、传输等能力插件。
- `ah-app`：读取 Profile、按依赖挂载插件并启动应用。
- `profiles/`：开发和生产组合配置；开发 Profile 可使用 mock，生产 Profile 禁止 mock。

插件之间通过 `ah-contracts` 声明的 Seam 通信。注册操作返回 `Effect`，销毁 Effect 即回滚注册；生产路径中的不支持能力必须显式返回错误，不使用静默 fallback。

## 快速开始

前置条件：Rust stable、Cargo，以及特定生产能力所需的外部服务或凭据。

```sh
cargo fmt --all --check
cargo build --workspace
cargo test --workspace
cargo run --offline -p ah-app --bin ah-app -- profiles/dev.toml
```

开发 Profile 使用确定性的 mock provider，适合本地启动和 Rust 协议冒烟，不代表生产可用性。生产 Profile 需要按 [docs/development.md](docs/development.md) 准备 OpenAI 凭据、Redis 和其他外部依赖：

```sh
cargo run --offline -p ah-app --bin ah-app -- profiles/prod.toml
```

## 文档

- [文档索引](docs/README.md)：阅读顺序和完整文档目录。
- [使用文档站点](website/README.md)：VitePress 本地开发、构建和预览入口。
- [架构约束](docs/architecture.md)：依赖方向、Seam、Effect、事件和 Profile 规则。
- [使用指南](docs/usage.md)：启动、编写插件和定义 Seam。
- [开发流程](docs/development.md)：测试、CI 和 Definition of Done。
- [能力地图](docs/capability-map.md)：历史 Python 能力到 Rust 插件的映射和当前状态。
- [测试规范](docs/testing.md)：Rust 回归、contract fixture、生产验证和覆盖率口径。

产品实现、默认测试和 CI 均不调用 Python。`differential/` 下的 Python 脚本仅保留为非门禁历史审计工具，不参与 Rust 构建或运行。

## 贡献

请先阅读 [AGENTS.md](AGENTS.md) 和 `docs/` 中的必读文档。新增能力应先定义契约，再实现 provider 和 consumer，并补齐挂载、解析、调用、卸载及适用的失败、取消、超时、恢复和持久化测试。提交信息遵循 Conventional Commits，例如：

```text
docs: clarify plugin contribution workflow
feat(memory): add persistent memory provider
```

提交前至少运行：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 许可证

本项目以 Apache License 2.0 发布，详见 [LICENSE](LICENSE)；补充声明见 [NOTICE](NOTICE)。
