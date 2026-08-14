# agent-harness

Rust 原生 agent harness,以高解耦插件架构为目标,设计思路参考 DeepSeek Harness
(Cordis: 服务注册表 + 类型化事件 + 可逆注册)与 openJiuwen agent-core 的行为面。

## 架构方向

```text
agent-harness/
  crates/
    ah-contracts/             契约层: Seam trait + 纯类型,零实现
    ah-hub/                   插件内核: ServiceRegistry + EventBus + Profile
    (规划) ah-plugins-*/      各域插件(provider / 引擎 / 工具)
    (规划) ah-app/            CLI + Web 入口
  profiles/                     组合配置: dev(mock) / prod(真实)
```

核心原则(对齐 DSH/Cordis):

- **无特权核心**:模型、工具、会话日志、agent 循环都是插件。
- **Seam 契约**:契约 crate 只声明接口(Service Definition / Provider / Consumer 三角),
  实现者依赖契约而非彼此。
- **类型化事件**:emit / waterfall / parallel / serial 四种分发。
- **可逆注册**:插件注册以 RAII guard 表达,卸载自动回滚。
- **日志即真相**:会话以 append-only 事件日志为唯一事实来源。
- **Profile 门禁**:生产 profile 不允许出现 mock 插件,CI 校验展开后的插件清单。

## 状态

脚手架阶段。当前仅 workspace + 两个基础 crate 的骨架。

## 构建

```sh
cargo build --workspace
cargo test --workspace
```
