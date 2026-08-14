# 内核语义入门(hub-primer.md)

> 等价 DSH 的 cordis-primer:解释 ah-hub 的机制语义。写插件/改内核前必读。

## 1. 服务注册表(ServiceRegistry)

- 存储:HashMap<ServiceKey, Arc<dyn Any>>,载荷是 Arc<S>(S 为具体类型或 trait 对象);
- 注册:ctx.register(key, Arc::new(provider) as Arc<dyn Trait>) —— 以 trait 对象为载荷;
- 查找:ctx.service::<dyn Trait>(&key),按相同 S 下转;
- 同键重复注册会覆盖(插件层 mount_all 会拒绝重复 provider,单点注册不拦);
- 注册返回 Effect,drop 即删除该键。

注意:register / on* 的接收者是 self: &Arc<Self>,因此 Effect 能持有 Arc 副本,
回滚闭包是 'static 的。

## 2. 可逆注册(Effect)

```rust
let _effect = ctx.register(key, service);   // 存活期间生效
drop(_effect);                              // 立即回滚
```

陷阱:**let _ = ... 会立即 drop 并回滚**。测试与插件中务必用具名绑定(let _e = ...)。

## 3. 插件(Plugin)

```rust
trait Plugin: Send + Sync {
    fn name(&self) -> &'static str;
    fn provides(&self) -> Vec<ServiceKey>;            // 本插件提供的键
    fn inject(&self) -> Vec<ServiceKey>;              // 依赖的键(apply 前必须已存在)
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError>;
}
```

mount_all 语义:

1. 组内重复 provider → DuplicateProvider;
2. 依赖拓扑排序(DFS,依赖在前)→ 环 → CycleDetected;
3. 按拓扑序 mount;某插件 inject 的键既不在组内也不在已注册集合 → MissingDependency;
4. 返回全部 Effect 的合集;drop 合集 = 卸载全部插件。

## 4. 事件总线(EventBus)

### 4.1 事件类型

事件实现 Event trait(要求 Send + Sync + Clone + 'static,ID 为稳定标识)。
分发按 TypeId 定位监听器列表。

### 4.2 四种分发

| 方法 | 语义 | 监听器签名 | 返回 |
| --- | --- | --- | --- |
| on / emit | 同步、顺序 | Fn(E) | () |
| on_serial / serial | 异步、顺序 await | Fn(E) -> Fut | () |
| on_parallel / parallel | 异步、并发 | Fn(E) -> Fut | () |
| on_waterfall / waterfall | 异步、next 链 | Fn(E, R, Next<R>) -> Fut | R |

异步监听器按值收事件(E: Clone,分发时逐监听器克隆)。

### 4.3 waterfall 精确语义

- 分发值 R 从 initial 开始,逐个监听器传递;
- 监听器收到 (event, value, next);调用 next.next(v) 把(可能修改的)v 委托下游;
- **不调用 next 即短路**:下游不再执行,当前返回值作为最终结果;
- 链尾的 next 是透传(identity);
- 实现:链从尾到头构建,每个节点包一个 ErasedNext(FnOnce),结果经 Box<dyn Any> 下转。

示例:

```rust
// 先注册的先执行;h1 委托后 +1,h2 委托后 *2;无监听器时返回 initial。
ctx.on_waterfall::<Ev, i32, _, _>(|_e, v, next| async move { next.next(v).await + 1 });
ctx.on_waterfall::<Ev, i32, _, _>(|_e, v, next| async move { next.next(v).await * 2 });
let r = ctx.waterfall(Ev, 1).await;   // (1*2)+1 = 3
```

## 5. Profile 组合

```toml
name = "dev"
[[bundles]]
id = "mock"
plugins = ["ah-plugins-mock"]
```

- 展开规则:bundle 顺序 + 插件名去重(plugin_names());
- 挂载顺序由依赖拓扑决定,不是配置顺序;
- 环境选择通过 profile 表达,不改代码;mock 门禁只拦生产 profile。

## 6. boot 流程(ah-app)

profile 加载 → 插件目录按名解析 → mount_all → ctx.service::<dyn Trait>(&key) 解析 seam
→ 调用。挂载后 ctx.service_keys() 可诊断已注册服务。
