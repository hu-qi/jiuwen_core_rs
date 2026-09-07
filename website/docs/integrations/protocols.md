# MCP 与 A2A

## MCP

Model Context Protocol 用于发现和调用外部工具。常见 Transport 包括 stdio 和 Streamable HTTP。

接入时验证：

- 子进程或 HTTP Session 生命周期
- 初始化和能力协商
- 工具列表与参数 Schema
- 超时、取消和 stderr
- 断线重连与协议错误
- 工具权限和返回值校验

## A2A

Agent-to-Agent 协议用于跨进程或跨服务提交任务、获取事件和取消执行。生产接入需要处理：

- 任务 ID 和幂等性
- 流式事件顺序
- 鉴权
- 取消和终态
- 重连、重放和重复事件
- 远端错误到本地错误的映射

MCP 或 A2A 连接成功只证明 Transport 可用；还需要验证工具或远端 Agent 的真实业务行为。
