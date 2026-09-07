# 开始使用

`agent-harness` 是 **openJiuwen 的 Rust 版本实现**，也是一个插件化 Agent Harness。内核提供服务注册、事件分发、插件生命周期和 Profile 组合；模型、工具、会话、工作流、团队、遥测等能力通过插件接入。

## 本地启动

在 `agent-harness` 目录执行：

```sh
cargo run -p ah-app -- profiles/dev.toml
```

开发 Profile 可以使用 Mock Provider，适合验证插件挂载、服务解析和命令路由。

## 推荐阅读顺序

1. [Harness 启动](/guide/harness)
2. [插件开发](/guide/plugin)
3. [服务契约与事件](/concepts/)
4. [Profile 组合](/guide/profile)
5. [外部集成](/integrations/)
6. [生产运维](/operations/)
7. [参考资料](/reference/)

## 插件开发入口

| 你要解决的问题 | 文档 |
| --- | --- |
| 先了解有哪些内置插件 | [内置插件概览](/guide/plugins) |
| 从零实现一个插件 | [插件简介与开发](/guide/plugin) |
| 查 `Plugin`、`Context`、`Effect` API | [Plugin API](/reference/plugin-api) |
| 接入 Catalog 和 Profile | [Catalog 与 Profile API](/reference/plugin-catalog) |
| 验证挂载、回滚和卸载 | [插件生命周期与测试](/guide/plugin-lifecycle) |

## 运行链

```text
Profile → ah-app catalog → dependency graph → Context::mount_all
→ ctx.service::<dyn Trait>() → invoke → drop Effects
```

## 适用边界

- 插件静态编译，启动时组合，关闭时释放。
- 插件之间通过 `ah-contracts` 和 `ah-hub` 通信，不依赖其他插件的具体类型。
- 生产路径中的不支持能力必须返回显式错误。
- Mock、schema 或本地 fallback 测试通过，不等于真实外部服务可用。
