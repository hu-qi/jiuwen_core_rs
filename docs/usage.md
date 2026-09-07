# 使用指南

> 面向插件作者和集成方。架构规则见 `architecture.md`,当前生产验证状态见 `README.md` 和
> `ROADMAP.md`。

## 本地启动

```sh
cargo run -p ah-app -- profiles/dev.toml
```

`dev.toml` 包含 `ah-plugins-mock`,用于无云凭据的开发冒烟。阶段二的 10 个新增插件已接入 `ah-app::plugin_catalog` 与 dev/prod Profile,并有 targeted mount/resolve/invoke/unmount 集成测试。Profile 插件数量不在本文手工复制,以实际解析结果为准。当前 Cargo workspace 可见 112 个 package。

```text
[boot] mounted services: [ServiceKey("llm"), ServiceKey("tools"), ...]
[llm] mock: ...
```

启动流程:

```text
Profile -> ah-app catalog -> 参数化插件 -> Context::mount_all -> ctx.service::<dyn Trait>()
```

生产 Profile:

```sh
cargo run -p ah-app -- profiles/prod.toml
```

`prod.toml` 不应包含 mock,但需要 OpenAI 凭据、本地 Redis 和若干外部能力。
当前仅有 mock exclusion/static composition 证据;当前 HEAD 尚无统一的无 mock production boot + `ApplicationRuntime::invoke` 验证。启动失败时应根据显式错误补齐依赖,不得回退到 mock 后仍视为生产验证通过。

## 编写插件

### 实现 seam provider

```rust
#[async_trait::async_trait]
impl ModelProvider for MyProvider {
    fn name(&self) -> &'static str {
        "my-provider"
    }

    async fn chat(&self, request: ModelRequest) -> Result<ModelResponse, ModelError> {
        // Real provider behavior.
    }
}
```

### 注册插件

```rust
impl Plugin for MyPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-my"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![ah_contracts::keys::LLM]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![ah_contracts::keys::CREDENTIALS]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn ModelProvider> = Arc::new(MyProvider::new(/* ... */));
        Ok(vec![ctx.register(ah_contracts::keys::LLM, provider)])
    }
}
```

规则:

- `provides` 声明服务输出,`inject` 声明依赖;
- consumer 解析 trait object,不导入具体 provider;
- 所有注册和后台资源必须绑定 Effect;
- 插件生产依赖只包含 hub/contracts 和外部基础库;
- 测试组装需要的具体插件放入 dev-dependencies。

### 加入 catalog 和 Profile

```toml
name = "my"

[[bundles]]
id = "my"
plugins = ["ah-plugins-my"]
```

同时在 `ah-app` catalog 注册稳定名称。新增配置字段需更新 `config-catalog.md` 和非法值测试。

## 定义 seam

一个完整 seam 包含:

1. `ah-contracts` 中的 trait、事件和纯类型;
2. 稳定 ServiceKey;
3. 至少一个 provider;
4. 至少一个通过 Context 解析 trait 的 consumer;
5. mount/resolve/invoke/unmount 测试;
6. 对应 capability-map 和 differential fixture。

只有 Service Definition 没有 provider/consumer,不能视为功能完成。

## ApplicationRuntime

应用层通过 trait seam 调用:

```rust
let application = ctx
    .service::<dyn ApplicationRuntime>(&APPLICATION)
    .ok_or("application service missing")?;

let result = application
    .invoke(AgentRequest {
        session_id: "session-1".into(),
        input: "inspect the workspace".into(),
        workflow: None,
        timeout_ms: Some(30_000),
        restore_checkpoint: None,
    })
    .await?;
```

当前支持命名 session、workflow 路由、checkpoint restore 和部分 controller 命令。执行中取消、
结构化终止错误、真实 iteration 统计、完整 memory/invoke rails 和 Python parity 仍为 partial。

## 事件

### emit

```rust
let effect = ctx.on::<MyEvent>(|event| {
    println!("observed: {:?}", event);
});
ctx.emit(MyEvent::new());
drop(effect);
```

### serial/parallel

```rust
let serial_effect = ctx.on_serial::<MyEvent, _, _>(|event| async move {
    consume_in_order(event).await;
});
let parallel_effect = ctx.on_parallel::<MyEvent, _, _>(|event| async move {
    consume_concurrently(event).await;
});
```

### waterfall

```rust
let effect = ctx.on_waterfall::<Decision, Decision, _, _>(
    |_event, value, next| async move {
        if should_deny(&value) {
            return Decision::Deny;
        }
        next.next(value).await
    },
);
```

Waterfall handler 不调用 `next()` 即短路。Effect 被 drop 后 listener 立即移除。

## Profile 组合

- Profile 是 Bundle 和插件名清单;
- 挂载顺序由 provides/inject 拓扑决定,不是 TOML 顺序;
- 同一 ServiceKey 的重复 provider 会失败;
- 缺失依赖和依赖环会失败;
- prod 不允许 mock;
- prod 无 mock不等于 prod 可完整启动。

## CLI

```sh
cargo run -q --bin ah-cli -- profiles/dev.toml
```

CLI 支持会话、团队、队列、工作区、web、code、RSI 等命令。dev profile 下模型由 mock 提供。
CLI smoke 证明二进制和命令路由可运行,不证明外部 provider 或 Python parity。

可运行的 Claude Code 风格开发工具示例位于 `example/ah-code-cli/`，使用 `ah-plugins-*` 真实插件链提供 REPL、文件读写、编辑、Shell、Session 和 Agent Loop：

```sh
cargo run -p ah-code-cli -- --workspace .
```

示例默认使用离线 Mock Provider；通过 `--model openai` 可切换真实 OpenAI-compatible Provider，缺少凭据时显式失败。

终端渲染插件 `ah-plugins-cli` 位于 `example/ah-code-cli/plugins/`，仅由该示例依赖；主项目 `ah-app` Catalog 和 dev/prod Profile 不注册它。

## 常见问题

- **解析不到服务**:检查插件是否在 Profile、名称是否在 catalog、provides/inject 是否一致;
- **重复 provider**:`mount_all` 会拒绝同一 ServiceKey 的两个 provider;
- **事件未触发**:确认 Effect 仍被持有;
- **prod 启动失败**:检查 OpenAI 凭据、Redis、MCP/外部命令和目录权限;
- **测试通过但状态仍 partial**:检查 production E2E 与 Python differential 是否完成;
- **需要具体插件组装测试**:将依赖放入 dev-dependencies,不要污染插件生产依赖。
