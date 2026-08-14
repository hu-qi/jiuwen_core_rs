# agent-harness 文档集

本文档集参考 DeepSeek Harness 的文档体系制定,目标有二:

1. **完整贯彻 DSH 架构理念**:无特权核心、Seam 契约、类型化事件、可逆注册、
   Profile 组合、日志即真相、mock 门禁;
2. **以完整实现 agent-core(Python)全部功能为目标**:能力地图将 Python 各域
   逐模块映射到 Rust 的 seam / 插件 / 工作包,作为开发与验收的单一依据。

## 文档清单与阅读顺序

| 文档 | 角色 | 读者 | 必读条件 |
| --- | --- | --- | --- |
| [architecture.md](architecture.md) | 架构约束:理念、分层、Seam/事件/Profile 契约、硬性规则 | 所有贡献者 | 改动任何 crate 之前
| [capability-map.md](capability-map.md) | 能力地图:agent-core 全功能 → seam/插件/工作包映射与验收标准 | 规划与验收 | 开始任何迁移工作之前
| [development.md](development.md) | 开发流程:搭建、工作包生命周期、测试、CI 门禁、提交规范 | 所有贡献者 | 提交代码之前
| [usage.md](usage.md) | 用户文档:插件开发、seam 定义、事件、profile 组合、运行 | 插件作者 / 集成方 | 编写第一个插件之前
| [agent-guide.md](agent-guide.md) | 生成参考:面向 AI 代理/代码生成的仓库指南与任务配方 | 编码代理 | 在仓库中生成任何代码之前

## 当前实现状态(截至框架提交)

- 已实现:ah-hub(ServiceRegistry / EventBus 四分发 / Plugin+拓扑挂载 / Profile)、
  ah-contracts(ServiceKey / Event / Seam / ModelProvider 示例)、ah-plugins-mock、
  ah-app(profile 启动端到端)。
- 规划:其余 seam 与插件(见 capability-map.md)。
- 本仓库禁止用文档声称完成度;完成度只以代码证据(测试 + 真实路径)为准。
