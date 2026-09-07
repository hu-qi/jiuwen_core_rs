# 术语表

**Agent**：接收目标，使用模型、工具和上下文完成任务的运行单元。

**Workflow**：由组件和连接组成的有向执行图。

**Component**：Workflow 中消费输入并产生输出的节点。

**Runner**：启动和协调 Agent、Workflow 或 Tool 执行的入口。

**Session**：一次或多次相关交互的状态边界。

**Context**：当前执行可访问的服务、配置和状态容器。

**Checkpoint**：用于中断后恢复执行的持久化状态。

**Provider**：某个服务契约的具体实现。

**Consumer**：通过契约调用服务的一方。

**Seam**：插件之间共享的稳定服务契约。

**Plugin**：向 Context 注册服务、事件监听器或运行能力的可组合单元。

**Profile**：声明 Harness Bundle 和插件组合的配置。

**Effect**：管理注册生命周期的可回滚句柄；释放时撤销对应注册。

**Mock**：只用于开发或测试的替代实现，不代表生产服务。

**Fallback**：主要实现失败后的替代路径。生产必需能力不得静默 fallback。

**Behavioral parity**：输入、输出、状态迁移、错误、取消、超时、恢复、持久化和协议行为等价。
