# Harness 模型、提示词与工具

模型插件负责把外部模型协议转换为稳定的模型 Seam；工具插件负责把受控能力暴露给 Agent Loop 或 Workflow。

## 边界

- Prompt 组装由调用方或 Prompt 插件负责。
- Provider 负责请求发送、响应解析和协议错误。
- Tool Consumer 负责选择和调用契约。
- 权限插件负责执行前后的安全决策。
- EventBus 负责记录可观察事件。

任何跨插件数据都应使用 `ah-contracts` 中的类型。不要通过共享具体 Provider 类型绕过 Seam。
