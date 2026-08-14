# agent-harness

Rust 原生 agent harness,以高解耦插件架构为目标,设计思路参考 DeepSeek Harness
(Cordis: 服务注册表 + 类型化事件 + 可逆注册)与 openJiuwen agent-core 的行为面。

## 架构方向

```text
agent-harness/
  crates/
    ah-contracts/     契约层: Seam trait + 事件契约 + 纯类型,零实现
    ah-hub/           插件内核: ServiceRegistry + EventBus + Plugin + Profile
    ah-plugins-mock/  Mock 插件集(仅 dev/test profile)
    ah-app/           boot 入口: 读取 profile → 组装插件 → 解析 seam
    (规划) ah-plugins-*/  各域插件(provider / 引擎 / 工具)
  profiles/           组合配置: dev(mock) / prod(真实)
```

核心原则(对齐 DSH/Cordis):

- **无特权核心**:模型、工具、会话日志、agent 循环都是插件。
- **Seam 契约**:契约 crate 只声明接口(Service Definition / Provider / Consumer 三角),
  实现者依赖契约而非彼此。
- **类型化事件**:emit / serial / parallel / waterfall 四种分发。
- **可逆注册**:插件注册以 RAII guard(Effect)表达,卸载自动回滚。
- **日志即真相**:会话以 append-only 事件日志为唯一事实来源(规划)。
- **Profile 门禁**:生产 profile 不允许出现 mock 插件,CI 校验展开后的插件清单。

## 当前框架能力(已实现)

- ah-hub:
  - ServiceRegistry:按键注册/查找 Seam trait 对象,注册返回 Effect,drop 自动反注册;
  - EventBus:emit(同步)/ serial(串行 await)/ parallel(并发)/ waterfall(next 链 + 短路);
  - Plugin trait + mount_all:依赖注入、拓扑排序挂载、循环依赖与重复 provider 检测;
  - Profile:TOML 组合配置(bundle 顺序 + 插件清单,去重展开)。
- ah-contracts:ServiceKey、Event、Seam 标记,以及 llm/tools/fs/shell/session/workflow/memory/retrieval/security/subagent/mcp/telemetry/credentials 等 seam 契约。
- ah-plugins-mock:MockModelProvider + 示例插件 MockPlugin。
- ah-plugins-mcp:真实 MCP stdio transport —— tokio 子进程 + newline-delimited JSON-RPC 2.0,真实 initialize 握手 / list_tools / call_tool / shutdown,并提供 mcp_call_tool 工具(集成测试用真实 fake server 子进程验证)。
- ah-plugins-telemetry:真实 telemetry 基础 —— 内存 span 记录 + JSONL 文件导出(export 追加写入并 flush,启动时读回已导出条数),挂 agent/step 与 tools/post-execute 监听生成真实 span;OTLP 导出留待后续。
- ah-plugins-credentials:真实凭据引用 —— 环境变量 provider(get/list 真实读 std::env,映射可配置:openai.api_key → OPENAI_API_KEY 等;set/remove 显式报错 env 只读),注册 credentials seam。
- ah-plugins-openai:真实 OpenAI 兼容 HTTP provider —— 配置可经 credentials seam 解析(openai.api_key/base_url/model 优先,再 fallback OPENAI_API_KEY/OPENAI_BASE_URL/OPENAI_MODEL 环境变量;两种来源都无 key 时挂载显式失败,不静默降级)。
- ah-app:cargo run -p ah-app 从 profiles/dev.toml 启动,挂载插件并调用 llm seam。

## 构建与运行

```sh
cargo build --workspace
cargo test --workspace
cargo run -p ah-app
```
