# CLI 命令

在 `agent-harness` 目录执行。

## 启动应用

```sh
cargo run -p ah-app -- profiles/dev.toml
cargo run -p ah-app -- profiles/prod.toml
```

## CLI 入口

```sh
cargo run -q --bin ah-cli -- profiles/dev.toml
```

CLI 的会话、团队、队列、工作区、Web、Code 和 RSI 命令由已挂载插件提供。dev Profile 下的模型可以是 Mock；这只证明命令路由和本地组合可运行。

## 验证门禁

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

单个插件或能力的验证应优先运行其 crate 范围内的测试，避免把本地 Mock 结果误判为生产验证。
