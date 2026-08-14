# 用户文档(usage.md)

> 面向插件作者与集成方:如何定义 seam、编写插件、组合 profile、使用事件。

## 1. 快速开始

```sh
cargo run -p ah-app
```

输出:

```text
[boot] profile: dev (ah-plugins-mock)
[boot] mounted services: [ServiceKey("llm")]
[llm] mock: mock reply to: hello from ah-app
```

启动流程:读取 profiles/dev.toml → 在插件目录中解析插件名 → mount_all(依赖拓扑排序)
→ 通过 ctx.service::<dyn ModelProvider>(&LLM_KEY) 解析 seam → 调用。

## 2. 编写一个插件

以 ah-plugins-mock 为模板,三步:

### 2.1 实现契约

```rust
#[async_trait::async_trait]
impl ModelProvider for MyProvider {
    fn name(&self) -> &'static str { "my-provider" }
    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        // 真实或确定性实现
    }
}
```

### 2.2 实现 Plugin trait 并注册服务

```rust
impl Plugin for MyPlugin {
    fn name(&self) -> &'static str { "ah-plugins-my" }
    fn provides(&self) -> Vec<ServiceKey> { vec![LLM_KEY] }
    fn inject(&self) -> Vec<ServiceKey> { vec![] }   // 依赖的服务键
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn ModelProvider> = Arc::new(MyProvider);
        Ok(vec![ctx.register(LLM_KEY, provider)])
    }
}
```

规则:

- provides 声明本插件提供的服务键;inject 声明依赖(挂载前必须已存在);
- apply 内所有注册必须返回 Effect,随插件卸载回滚;
- 插件 crate 只依赖 ah-hub + ah-contracts,不依赖其他插件。

### 2.3 加入 profile

```toml
# profiles/my.toml
name = "my"

[[bundles]]
id = "my"
plugins = ["ah-plugins-my"]
```

并在 ah-app 的插件目录中注册名称 → 插件对象映射。

## 3. 定义一个新 seam

1. 在 ah-contracts 中定义 trait(继承 Seam 标记)+ 纯类型;
2. 定义稳定 ServiceKey(如 `ctx.store` 对应 ServiceKey::new("store"));
3. 实现:至少一个 Provider(插件)+ 一个 Consumer(消费方)才构成完整 seam;
4. 契约层不允许出现实现。

## 4. 事件使用

### 4.1 emit(同步观察)

```rust
let _effect = ctx.on::<MyEvent>(|event| {
    println!("observed: {}", event.id());
});
ctx.emit(MyEvent::new());
```

### 4.2 serial / parallel(异步)

```rust
let _e = ctx.on_serial::<MyEvent, _, _>(|event| async move {
    // 顺序副作用
});
let _e = ctx.on_parallel::<MyEvent, _, _>(|event| async move {
    // 并发执行
});
ctx.serial(MyEvent::new()).await;
ctx.parallel(MyEvent::new()).await;
```

### 4.3 waterfall(委托链 / 短路)

```rust
// 决策链:任一监听器不调用 next 即短路。
let _e = ctx.on_waterfall::<Decision, Decision, _, _>(|_event, value, next| async move {
    if let Some(deny) = policy(&value) {
        return deny;                    // 短路:下游不再执行
    }
    next.next(value).await              // 委托下游
});
let final_decision = ctx.waterfall(Decision::new(), Decision::Allow).await;
```

## 5. Profile 组合

- profile = 有序 bundle + 插件清单;插件名去重展开;
- 挂载顺序由依赖拓扑决定,不是文件顺序;
- 环境选择通过 profile 表达,不改代码:
  - dev.toml:全 mock(开发/测试);
  - prod.toml(规划):全真实 provider,mock 门禁校验;
  - hybrid:按需混合。

## 6. 扩展点速查(规划中逐步开放)

| 想做什么 | 挂在哪里 |
| --- | --- |
| 加模型 provider | llm seam |
| 加模型可见能力 | tools seam,其 schema 进入 prompt 组装 |
| 加 shell 执行 | shell seam |
| 加文件系统策略 | fs seam 或 fs/* 事件 |
| 加人类命令 | 命令注册表(规划) |
| 拦截请求/工具/回合 | agent/*、tools/* 事件(waterfall) |
| 加后台任务 | jobs 服务(规划) |
| 加会话持久状态 | 会话事件日志(规划) |

## 7. 常见问题

- **解析不到服务**:检查 provides/inject 是否一致、插件是否已挂载(ah-app 打印 mounted services);
- **服务被覆盖**:同键重复注册会覆盖,插件层 mount_all 会拒绝重复 provider;
- **事件没触发**:监听器 Effect 被立即 drop 会反注册——用具名绑定持有 guard;
- **生产不能用 mock**:profile 门禁会拒绝 ah-plugins-mock 进入生产。

## 交互 CLI(ah-cli)

`cargo run -q --bin ah-cli -- <profile>`(默认 profiles/dev.toml)启动交互式 CLI;
会话/团队/队列/工作区真实持久化在当前目录 .agent-harness/ 下。

```text
  <task>                    在当前会话运行一个任务(真实 ReAct 循环)
  /new <id> | /use <id>     新建/切换会话
  /fork <from> <to>         分叉会话
  /teams create <id> <name> 创建团队(SQLite 持久化)
  /teams run <team> <task>  建任务并真实委派 subagent 执行
  /teams tasks <team>       列出团队任务
  /teams msg <team> <from> <content...>
  /teams msgs <team>        团队消息(经 queue seam)
  /rsi round <n> <seed>     跑一轮 RSI 评测(数据集生成 + 真实执行 + 评估)
  /rsi run <n> <seed>      多轮优化编排(评测→精化→checkpoint 续跑)
  /workspace goals / goal add <id> <title...> / goal done <id>
  /web fetch <url>          真实 HTTP GET
  /queue publish <channel> <json> | /queue consume <channel>
  /code run <code>          真实 python3 执行
```

e2e 冒烟测试:crates/ah-app/tests/cli_smoke.rs 用真实构建的二进制验证上述子命令。
