# 模块依赖图

> 当前代码 HEAD:`7404142`;`cargo metadata --no-deps` 实际包含 112 个 workspace package。仓库中有 112 个 Cargo manifest;阶段一已将 10 个新增插件加入 workspace members,阶段二已完成 catalog/Profile 接线与 targeted mount/resolve/invoke/unmount 验证。
> crate 数量不在此处手工维护;以 Cargo workspace 为准,并区分“有 manifest”“属于 workspace”“已接入 catalog/Profile”。

## 稳定分层

```text
ah-app (composition root)
  ├── ah-hub
  ├── ah-contracts
  └── ah-plugins-* concrete providers

ah-plugins-* (provider or consumer)
  ├── ah-hub
  └── ah-contracts

ah-hub
  └── ah-contracts

ah-contracts
  └── external foundational crates only
```

- `ah-app` 是唯一正常依赖大量具体插件的组装层;
- 插件生产代码只通过 seam trait 和 ServiceKey 协作;
- 插件测试可在 `[dev-dependencies]` 引入其他具体插件;
- `ah-hub` 只提供 Registry、EventBus、Plugin、Effect、Profile 和依赖拓扑;
- `ah-contracts` 只提供 trait、事件和纯数据类型,不得包含 provider 实现。

## 核心运行链

```text
Profile
  -> ah-app::plugin_catalog
  -> Context::mount_all
  -> credentials / llm / tools / sessions / context / controller
  -> agent-control / model-backup-policy
  -> agent-loop
  -> application
```

关键 crate:

| 角色 | crate | 主要 seam |
| --- | --- | --- |
| 契约 | `ah-contracts` | 全部 Service Definition、Event、Effect 类型 |
| 内核 | `ah-hub` | Context、Registry、EventBus、Plugin、Profile |
| 组装 | `ah-app` | catalog、Profile 参数解析、boot、CLI |
| 控制 | `ah-plugins-agent-control` | interrupt、callback manager |
| 循环 | `ah-plugins-agent-loop` | `AgentLoopRuntime` consumer/provider |
| 应用 | `ah-plugins-application` | `ApplicationRuntime`,LLM/workflow/controller 路由 |
| 能力 | `ah-plugins-ability` | AbilityManager |
| 备份 | `ah-plugins-model-backup` | backup provider/policy |
| 会话 | `ah-plugins-session-log` | SessionLog、SessionManager |
| 工作流 | `ah-plugins-workflow` | WorkflowEngine |
| 控制器 | `ah-plugins-controller` | Controller、TaskSnapshotStore |

当前 `ah-app::agent_and_manager` 返回 `dyn AgentLoopRuntime`;宿主不再依赖具体 AgentLoop。
但 Agent Loop 的 Python 行为对等、生产验证和完整生命周期仍未完成,见 `ROADMAP.md` P1-02/P1-03 及能力映射。

## 能力域

具体插件按能力域组织,完整列表以 workspace `Cargo.toml` 为准:

- 模型与凭据:mock、openai、anthropic、oauth、credentials、model-catalog、model-backup;
- Agent 与编排:agent-loop、application、agent-control、ability、workflow、pregel、controller、runner;
- 工具与安全:tools、sysop、code、web、lsp、mcp、rails、security、sandbox、worktree;
- 上下文与数据:context、memory、retrieval、rerank、store、queue、kv-cache、resources;
- 多 Agent:subagent、subagents、teams、transport、external、team-*;
- 演进:operator、optimizer、trainer、tune、evolving、RSI 及 evaluator/curator/learner 插件;
- 工程与遥测:CLI、CI、git、telemetry、reliability、manifest、prompt-builder、skill-creator。

能力域中的“有 crate”不表示 Python parity done。状态见 capability/parity audit。

## 生产依赖规则

允许:

```toml
[dependencies]
ah-contracts = { workspace = true }
ah-hub = { workspace = true }

[dev-dependencies]
ah-plugins-mock = { workspace = true }
ah-plugins-session-log = { workspace = true }
```

除 `ah-app` 外不允许:

```toml
[dependencies]
ah-plugins-other = { workspace = true }
```

当前已有 `ah-app/tests/plugin_isolation.rs` 自动检查生产依赖隔离;新增插件已完成 workspace/catalog/Profile 接线和阶段二 mount/resolve/invoke/unmount 测试,后续仍需 production boot 与完整行为对等。

## 生命周期边界

插件通过 `apply()` 注册服务和事件,并返回 `Vec<Effect>`。Effect drop 必须撤销:

- ServiceRegistry 注册;
- EventBus listener;
- 后台 Tokio task;
- 子进程;
- socket/transport;
- 临时资源和 watcher。

Registry/EventBus 已有基础可逆测试;批量挂载中途失败原子性已由 `ah-hub` 回归测试覆盖,后台资源释放和 provider 热替换仍需系统测试,见 P1-09。

## 新增 crate 检查表

1. 在 workspace members 和 `[workspace.dependencies]` 登记;
2. 生产依赖符合分层规则;
3. 在 capability-map 登记能力映射;
4. 在 `ah-app` catalog 和需要的 Profile 中接线;
5. 增加 mount/resolve/invoke/unmount 测试;
6. 更新相关 config/event/persistence catalog;
7. 记录 implementation、production、parity 三维状态;
8. 运行聚焦测试和受影响 workspace 门禁。
