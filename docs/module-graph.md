# 模块依赖图(module-graph.md)

> 等价 DSH 的 module-graph:登记 crate 布局与依赖规则。新增 crate 时必须更新本文件。

## 1. 当前 crate 依赖

```text
ah-app ──> ah-hub ──> ah-contracts
   │          │
   └─> ah-plugins-mock ──> ah-hub, ah-contracts
```

依赖方向(单向、禁止环):

- ah-contracts:零依赖(仅 async-trait);
- ah-hub → ah-contracts;
- ah-plugins-* → ah-hub + ah-contracts(插件之间禁止互依赖);
- ah-app → ah-hub + 插件(组装)。

## 2. 规划:完整插件布局(源自 capability-map.md)

```text
ah-contracts                21 个 seam(见 capability-map.md §1)
ah-hub                      内核
ah-plugins-mock             mock 全家桶(dev/test)
ah-plugins-core-*           common/application/runner/single-agent/context 等
ah-plugins-workflow-engine  工作流/图/controller/operator(迁移 rp301)
ah-plugins-session-log      会话事件日志(迁移 state.rs/persist.rs)
ah-plugins-sysop-*          fs/shell/code/sandbox(迁移 sys_operation.rs)
ah-plugins-harness-*        tools/rails/subagents/cli/workspace
ah-plugins-teams            团队运行时(迁移 residual.rs)
ah-plugins-evolving         agent_evolving 域
ah-plugins-rsi              RSI + auto_harness
ah-plugins-store-*          redis/pulsar/gaussdb/elasticsearch/milvus/chroma
ah-plugins-transport-*      mcp/a2a
ah-plugins-otel             telemetry
ah-plugins-openai           第一个真实 LLM provider(迁移 OpenAiCompatibleClient)
ah-plugins-devtools         dev_tools 域
ah-plugins-symphony         symphony
ah-app                      boot 入口
```

## 3. 新增 crate 清单

1. 在 workspace Cargo.toml members + [workspace.dependencies] 登记;
2. 在本文件 §1/§2 更新依赖关系;
3. 在 capability-map.md 登记对应能力;
4. 遵循命名:插件为 ah-plugins-<域>[-<子域>],其余为 ah-<模块>;
5. CI 门禁会校验 fmt / clippy / test / mock 门禁。
