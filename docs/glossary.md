# 术语表(glossary.md)

本文统一全文档集的术语含义。新文档必须使用本文定义,不另造同义说法。

## 架构术语

| 术语 | 定义 |
| --- | --- |
| Seam | 可替换能力的契约(Service Definition / Provider / Consumer 三角)。如 llm、tools、store。 |
| Service Definition | Seam 的接口部分(如 ModelProvider trait),位于 ah-contracts。 |
| Service Provider | 接口的实现(插件),位于 ah-plugins-*。 |
| Consumer | 消费方,通常是模型可见工具或 agent 循环。 |
| ServiceKey | 类型化服务键,等价 Cordis 的 ctx.<key>;插件与消费方共享的稳定契约。 |
| Context | 插件上下文:ServiceRegistry + EventBus 的组合,插件挂载与查找的入口。 |
| Plugin | 挂载到 Context 的单元,声明 provides/inject,apply 内注册服务与事件。 |
| Effect | 可逆注册的 RAII guard;drop 即回滚。 |
| 契约零实现 | ah-contracts 只声明接口与纯类型,不出现 Mock/Unsupported/fallback/业务逻辑。 |
| mock 门禁 | 生产 profile 展开后不得包含 ah-plugins-mock 插件;CI 校验。 |
| 日志即真相 | 任何到达模型请求的输入都能从 append-only 会话事件日志重建。 |

## 事件术语

| 术语 | 定义 |
| --- | --- |
| emit | 同步、按注册顺序通知(观察)。 |
| serial | 异步、按注册顺序逐个 await(顺序副作用)。 |
| parallel | 异步、并发执行全部监听器(扇出)。 |
| waterfall | 异步、next() 委托链;不调用 next 即短路。 |
| Next | waterfall 的委托句柄;next(value) 把值传给下游。 |

## 组合与验收术语

| 术语 | 定义 |
| --- | --- |
| Profile | TOML 组合配置:有序 bundle + 插件清单。 |
| Bundle | 一组插件的分发单元(bundle.id + bundle.plugins)。 |
| 差分契约 | 与 agent-core(Python)行为对等的语言中立测试契约(fixtures)。 |
| golden fixture | 每个 seam 的成功/非法输入/超时/取消/恢复/序列化基准样例。 |
| done / partial / missing / excluded | 能力状态:真实路径+测试 / 有类型缺语义 / 无实现 / 正式排除。 |
| 真实路径 | 非 mock、非 fallback、非 unsupported 的生产执行路径。 |
