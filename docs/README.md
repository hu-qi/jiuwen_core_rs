# agent-harness 文档集

本文档集参考 DeepSeek Harness 的文档体系制定,目标有二:

1. **完整贯彻 DSH 架构理念**:无特权核心、Seam 契约、类型化事件、可逆注册、
   Profile 组合、日志即真相、mock 门禁;
2. **以完整实现 agent-core(Python)全部功能为目标**:能力地图将 Python 各域
   逐模块映射到 Rust 的 seam / 插件 / 工作包,作为开发与验收的单一依据。

## 文档清单与阅读顺序

### 必读(改动任何代码之前)

| 文档 | 角色 | 读者 |
| --- | --- | --- |
| [architecture.md](architecture.md) | 架构约束:理念、分层、硬性规则 | 所有贡献者 |
| [hub-primer.md](hub-primer.md) | 内核语义入门:注册表/事件/Effect/Plugin/Profile | 写插件或改内核者 |
| [capability-map.md](capability-map.md) | 能力地图:agent-core 全功能 → seam/插件/工作包 | 规划与验收 |
| [agent-guide.md](agent-guide.md) | 生成参考:面向 AI 代理的任务配方与验证命令 | 编码代理 |

### 参考(按需)

| 文档 | 角色 |
| --- | --- |
| [development.md](development.md) | 开发流程:工作包生命周期、CI 门禁、提交规范 |
| [testing.md](testing.md) | 测试与对等验证:契约 fixtures、差分契约、覆盖率 |
| [usage.md](usage.md) | 用户文档:插件编写、seam 定义、事件、profile |
| [glossary.md](glossary.md) | 术语表:全文档集统一术语 |
| [event-catalog.md](event-catalog.md) | 事件目录:事件 × 模式 × 生产者 × 消费者 |
| [config-catalog.md](config-catalog.md) | 配置目录:profile 与插件配置字段 |
| [module-graph.md](module-graph.md) | 模块依赖图:crate 布局与新增规则 |

## 规划文档(能力落地后再写)

以下文档描述尚未实现的子系统。为避免"文档先于代码"的空转,只在对应能力
达到 partial 后创建,并在本表登记:

| 规划文档 | 触发条件 | 内容 |
| --- | --- | --- |
| tool-catalog.md | tools seam 落地 | 工具注册表、工具分类 |
| tool-execution-pipeline.md | 工具执行管线 | 执行管线、鉴权、超时、回滚 |
| agent-lifecycle.md | agent-loop 落地 | turn/step 生命周期、事件序列 |
| persistence-catalog.md | session log 落地 | 持久化格式与版本策略 |
| cookbook.md | 插件数 > 3 | 面向插件作者的扩展配方集 |

## 当前实现状态(截至框架提交)

- 已实现:ah-hub(ServiceRegistry / EventBus 四分发 / Plugin+拓扑挂载 / Profile)、
  ah-contracts(ServiceKey / Event / Seam / Effect / llm+tools 两个 seam)、
  ah-plugins-mock(MockModelProvider + MockToolRegistry + Echo/Add 工具)、
  ah-app(profile 启动端到端,同时消费 llm 与 tools seam)。
- 规划:其余 seam 与插件(见 capability-map.md)。
- 本仓库禁止用文档声称完成度;完成度只以代码证据(测试 + 真实路径)为准。

## 文档维护纪律

1. 新文档在本文档集登记后生效;
2. 状态变更必须附实现路径与测试证据,禁止只改文档不改代码或反之;
3. 术语必须使用 glossary.md 定义;
4. 规划文档不在能力落地前创建(防止空转)。

