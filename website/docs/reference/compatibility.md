# 能力与实现状态

本页只描述 `agent-harness` 的能力判断口径。详细状态以仓库中的能力地图、测试证据和当前代码为准。

## 状态含义

| 状态 | 含义 |
| --- | --- |
| `done` | 实现、测试和适用的真实路径证据均满足验收口径 |
| `partial` | 存在局部实现，但生产协议、持久化、失败或恢复路径仍不完整 |
| `missing` | 没有可运行实现，或只有显式 unsupported 路径 |
| `excluded` | 明确不属于 Harness 产品范围，并记录了理由 |

## 验收原则

- 同一个 ServiceKey 的重复 Provider 必须失败。
- 缺失依赖和依赖环必须失败。
- 挂载失败必须回滚之前的注册。
- 释放 Effect 后服务和监听器必须消失。
- 生产 Profile 不得包含 Mock 插件。
- 真实外部服务不可用时必须显式报错，不得静默降级。

当前能力映射见 `agent-harness/docs/capability-map.md`；事件、配置和持久化目录见 `agent-harness/docs/*-catalog.md`。
