# 外部集成

openJiuwen 可以连接模型 Provider、工具协议、存储系统和可观测性平台。先完成最小本地执行，再逐个加入外部依赖。

## 集成目录

| 类型 | 能力 | 进入 |
| --- | --- | --- |
| 模型 | OpenAI、Anthropic、OpenAI-compatible | [模型 Provider](/integrations/providers) |
| 工具与 Agent 协议 | MCP、A2A | [MCP 与 A2A](/integrations/protocols) |
| 状态与检索 | Redis、关系数据库、向量库、对象存储 | [存储与 Checkpoint](/integrations/storage) |
| 观测 | OpenTelemetry、OTel Collector、Langfuse | [可观测性](/integrations/observability) |

## 上线前检查

1. 记录协议版本和服务端版本。
2. 把凭据放在受控环境变量或密钥服务中。
3. 为连接、请求和关闭设置超时。
4. 验证重连、取消、限流和错误映射。
5. 禁止外部服务失败后静默切换到 Mock 或本地内存。
6. 对模型输入、工具参数和日志实施脱敏。

::: tip 建议
每次只接入一个外部系统，并保留一个不依赖网络的最小冒烟路径。这样可以区分业务编排错误和基础设施错误。
:::
