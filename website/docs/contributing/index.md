# 参与贡献

本网站只接受 `agent-harness` 的代码、插件、测试和文档变更。

## 代码变更

- 先阅读 `AGENTS.md`、架构约束和 Hub 语义文档。
- 新能力先定义 Contract，再实现 Provider、Consumer 和失败路径。
- 插件只通过 `ah-contracts` 和 `ah-hub` 通信。
- 注册、监听器和后台资源必须绑定 Effect。
- 不提交凭据，不把 Mock 伪装成生产实现。

## 验证

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

新增插件还应验证 mount、resolve、invoke、unmount，以及缺失依赖、重复 Provider、取消、超时和清理行为。

## 文档变更

文档描述当前 Harness 行为、前置条件、错误、生命周期和验证方式。架构取舍放入决策记录；事件、配置、持久化和模块清单链接到其权威目录。
