# Harness 启动

## 开发 Profile

```sh
cargo run -p ah-app -- profiles/dev.toml
```

开发 Profile 使用确定性的 Mock Provider，验证目标是：

- Profile 可以被解析。
- Catalog 可以找到插件。
- 依赖拓扑可以完成挂载。
- ServiceKey 可以解析到契约对象。
- 关闭时 Effect 能够撤销注册。

## 生产 Profile

```sh
cargo run -p ah-app -- profiles/prod.toml
```

生产启动需要真实模型凭据、Redis 以及 Profile 中声明的其他外部依赖。启动失败时应修复缺失配置或服务，不得自动改用 Mock。

## 诊断挂载

启动过程遵循：

```text
Profile → plugin catalog → parameterized plugins → mount_all → service resolve
```

服务解析不到时，依次检查插件名称、`provides`、`inject`、ServiceKey 和 Profile 清单。

## 关闭

保留每个 `Effect` 的具名绑定，直到插件关闭完成。`let _ = ctx.register(...)` 会立即释放 Effect 并撤销注册。
