# 插件简介与开发

插件是 `agent-harness` 的运行能力单元。它在 `Context` 上注册服务、事件监听器和其他运行资源，并通过 `Effect` 管理这些注册的生命周期。

## 为什么使用插件

插件把三个角色分开：

```text
ah-contracts  定义契约
ah-plugins-*  提供实现
ah-app / 其他插件  组合并消费契约
```

因此，模型 Provider、工具注册表、工作流引擎、Session 日志、团队协调、遥测和安全护栏都可以独立替换和测试。

## 一个插件的最小组成

1. 一个稳定的插件名称。
2. `provides()` 声明输出的 ServiceKey。
3. `inject()` 声明挂载前必须存在的服务。
4. `apply()` 注册服务或监听器，并返回所有 Effect。
5. 在应用 Catalog 和 Profile 中注册名称。
6. 编写挂载、解析、调用和卸载测试。

## 完整最小示例

下面的示例定义一个 Seam、实现 Provider、挂载插件、调用服务并验证卸载。

```rust
use std::sync::Arc;

use ah_contracts::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::Context;
use ah_hub::plugin::{DynPlugin, Plugin, PluginError};

const GREETER: ServiceKey = ServiceKey::new("greeter");

trait Greeter: Seam {
    fn greet(&self, name: &str) -> String;
}

struct GreeterProvider;

impl Seam for GreeterProvider {}

impl Greeter for GreeterProvider {
    fn greet(&self, name: &str) -> String {
        format!("hello, {name}")
    }
}

struct GreeterPlugin;

impl Plugin for GreeterPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-greeter"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![GREETER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let provider: Arc<dyn Greeter> = Arc::new(GreeterProvider);
        Ok(vec![ctx.register(GREETER, provider)])
    }
}

fn main() {
    let ctx = Context::new();
    let plugins: Vec<DynPlugin> = vec![Arc::new(GreeterPlugin)];
    let effects = ctx.mount_all(plugins).expect("mount greeter");

    let greeter = ctx
        .service::<dyn Greeter>(&GREETER)
        .expect("greeter service");
    assert_eq!(greeter.greet("developer"), "hello, developer");

    drop(effects);
    assert!(!ctx.has_service(&GREETER));
}
```

插件 crate 的生产依赖保持最小：

```toml
[dependencies]
ah-contracts = { workspace = true }
ah-hub = { workspace = true }
```

测试需要组合其他插件时，把它们放在 `[dev-dependencies]`，不要形成插件生产依赖链。

## 接入仓库

1. 在根 `Cargo.toml` 把新 crate 加入 workspace members 和 workspace dependencies。
2. 在 `ah-app::plugin_catalog` 注册 `"ah-plugins-greeter"` 和 `DynPlugin` 对象。
3. 在适用的 `profiles/*.toml` Bundle 中加入插件名。
4. 如果新增配置字段，更新 `docs/config-catalog.md` 和非法值测试。
5. 编写 mount、resolve、invoke、unmount 集成测试。

## 下一步

- [内置插件概览](./plugins)
- [Plugin API](../reference/plugin-api)
- [Catalog 与 Profile API](../reference/plugin-catalog)
- [插件生命周期与测试](./plugin-lifecycle)
- [Service 与 Seam](../concepts/services)
