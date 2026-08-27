# 架构约束(architecture.md)

> 权威文档:改动任何 crate 之前必读。本文档的约束是**硬性规则**,不是建议。
> 参考对象:DeepSeek Harness(DSH)+ Cordis;目标对象:openJiuwen agent-core(Python)的完整功能。

## 1. 目标与非目标

### 目标

- **行为对等**:与 agent-core 的输入、输出、状态迁移、错误、取消、超时、恢复、
  持久化、外部协议行为对等;
- **完整功能覆盖**:agent-core 的 core / harness / agent_teams / agent_evolving / rsi /
  extensions / dev_tools 全部能力都必须在 Rust 插件体系中有对应实现(见 capability-map.md);
- **DSH 架构理念**:无特权核心、Seam 契约、类型化事件、可逆注册、Profile 组合、
  日志即真相、mock 门禁。

### 非目标(正式排除)

- Python import 路径兼容(不提供 openjiuwen.* 的 Rust 可导入等价物);
- Python 对象模型兼容(继承、metaclass、协程对象、对象同一性)。

## 2. 分层结构

```text
agent-harness/
  crates/
    ah-contracts/     契约层:Seam trait + Event trait + 纯类型,零实现
    ah-hub/           插件内核:ServiceRegistry + EventBus + Plugin + Profile
    ah-plugins-*/     插件:provider / 引擎 / 工具(每个插件一个 crate 或按域分组)
    ah-app/           boot 入口:读取 profile → 组装插件 → 解析 seam
  profiles/           组合配置:dev(mock)/ prod(真实)/ hybrid
  docs/               本文档集
```

**依赖方向(单向)**:`ah-app` → `ah-hub` → `ah-contracts`;`ah-plugins-*` 依赖
`ah-hub`(挂载 API)+ `ah-contracts`(契约);插件之间**禁止**直接依赖彼此的具体类型。

## 3. 核心机制(DSH 理念的 Rust 落地)

### 3.1 无特权核心

模型、工具、会话日志、agent 循环、rails、子代理……全部是插件,没有特权实现。
内核(ah-hub)只提供组合机制,不内置任何业务实现;唯一的例外是 `plugin-mock` 家族
(它们也必须是普通插件,只是被 profile 门禁排除在生产之外)。

### 3.2 Seam 契约三角

每个 Seam 由三方构成,缺一不可:

| 角色 | 定义 | 位置
| --- | --- | --- |
| Service Definition | 契约接口(如 `ModelProvider` trait) | `ah-contracts`
| Service Provider | 接口实现(插件) | `ah-plugins-*`
| Consumer | 消费方(常为模型可见工具/agent 循环) | 任意插件

规则:
- 契约 crate **零实现**,不允许出现 `Mock` / `Unsupported` / fallback;
- 消费方只通过 `ctx.service::<dyn Trait>(&ServiceKey)` 解析服务,不 import 实现;
- 服务键(`ServiceKey`)是稳定的公开契约,插件与消费方共享(`LLM_KEY` 即示例)。

### 3.3 类型化事件

事件是扩展点。四种分发模式(已实现,见 ah-hub):

| 模式 | 语义 | 用途示例
| --- | --- | --- |
| `emit` | 同步、按注册顺序通知 | 日志、遥测观察
| `serial` | 异步、按注册顺序逐个 await | 顺序副作用(审计链)
| `parallel` | 异步、并发执行全部 | 并行工具执行
| `waterfall` | 异步、`next()` 委托链,可短路 | `tools/pre-execute`(rails 决策链)、`tools/post-execute`

规则:
- 事件实现 `Event` trait(要求 `Clone`,`ID` 为稳定标识);
- waterfall 监听器**必须调用 `next()`** 才委托下游;不调用即短路(决策类监听器设计如此);
- 事件类型定义在 `ah-contracts`(跨插件共享)或插件自身(仅域内)。

### 3.4 可逆注册(Effect)

所有注册(服务、事件监听器)都返回 `Effect` RAII guard;guard drop 即回滚。
插件 `apply` 返回其注册产生的 `Effect` 列表,卸载插件时全部回滚。

### 3.5 Profile 组合与 mock 门禁

- `Profile`(TOML)= 有序 bundle 列表 + 插件清单,展开为挂载顺序;
- 挂载顺序由依赖拓扑决定(`inject`/`provides`),不是配置文件里写的顺序;
- **mock 门禁**:生产 profile 展开后不得包含 `plugin-mock` 插件;CI 校验(见 development.md);
- 同一个插件实例可被多个 profile 复用;环境选择通过 profile/overlay 表达,不改代码。

### 3.6 日志即真相(session log)

会话子系统(ah-plugins-session-log)已实现,采用 DSH 原则:**模型可见即已记录**。
任何到达模型请求的输入都必须能从 append-only 会话事件日志(JSONL)重建;
新的模型可见输入必须对应一个新的会话事件类型(session/event)。

## 4. 硬性规则(可执行约束)

1. **契约零实现**:`ah-contracts` 不出现 `Mock*` / `Unsupported*` / fallback / 业务逻辑。
2. **插件隔离**:插件 crate 只依赖 `ah-hub` + `ah-contracts`;禁止互相 import 具体类型。
3. **注册必可逆**:任何注册通过返回 `Effect` 表达;禁止无 guard 的裸注册。
4. **mock 显式化**:所有 mock/fallback 实现位于 `ah-plugins-mock`(或明确标注的 test 插件),
   生产 profile 选不到。
5. **真实路径不伪装**:`unsupported` 必须显式返回错误,禁止静默用 in-memory/本地代替;
   禁止 `todo!()`/`unimplemented!()` 出现在生产路径(用显式错误或类型表达)。
6. **waterfall 语义**:监听器不调用 `next()` 即短路;文档标注每个 waterfall 事件的语义。
7. **行为对等标准**:本地/mock 测试通过 ≠ 完成;完成 = 生产路径真实执行 + 差分契约通过。
8. **版本/ABI**:跨插件边界的序列化契约(进程插件 JSON-RPC 等)一经发布即为 API,需版本化。

## 5. 依赖隔离审计

生产插件的 `[dependencies]` 只能包含 `ah-contracts`、`ah-hub` 及通用第三方库；其他 `ah-plugins-*` 只能出现在 `[dev-dependencies]`，用于集成测试装配。生产源码的跨能力调用必须通过 contracts trait 与 `ServiceKey`，不得 `use ah_plugins_*` 或引用其他插件的具体类型。提交前应分别检查 Cargo manifest 和 `src` 中非 `#[cfg(test)]` 区域。

当前审计结论：`ah-plugins-agent-loop`、`ah-plugins-application`、`ah-plugins-model-backup` 均符合该规则；agent-loop/application 的具体插件装配仅存在于测试代码。完整工作区的生产依赖扫描未发现具体插件依赖。

## 6. 迁移资产(来自 agent-core_rs 的现成实现)

| 资产 | 迁移为 | 状态
| --- | --- | --- |
| sys_operation(fd 级受限工作区) | ah-plugins-sysop(本地 fs/shell 执行) | ✅ 已迁移
| workflow/controller 状态机 | ah-plugins-workflow(引擎) + ah-plugins-controller | ✅ 已迁移
| session/state/persist | ah-plugins-session-log(JSONL 日志 + 多会话) | ✅ 已迁移
| 团队任务板/日志/预算 | ah-plugins-teams(SQLite 持久化 + swarmflow) | ✅ 已迁移
| OpenAI 兼容 HTTP 客户端 | ah-plugins-openai + ah-plugins-anthropic(真实协议) | ✅ 已迁移
| 本地受限 shell 执行 | ah-plugins-sysop | ✅ 已迁移

## 6. 实现状态标注规则

- 文档不得声称完成度;完成度只以代码证据为准;
- 每个能力在 capability-map.md 中标注:done(测试+真实路径)/ partial / missing / excluded;
- 状态变更必须附实现路径与测试证据(类似 agent-core_rs 的 RUST_CAPABILITY_MATRIX 纪律,但以本仓库代码为准)。
