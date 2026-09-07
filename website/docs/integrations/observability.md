# Harness 可观测性

Harness 的遥测插件可以通过 OpenTelemetry 导出运行事件和 Trace。典型链路：

```text
Harness Application → OTLP gRPC / HTTP → OTel Collector → Backend
```

## 本地验证

启动 Collector 后，使用 dev Profile 运行一个最小请求，确认以下信息可见：

- Session 或执行 ID
- Plugin 和 ServiceKey
- 模型请求或工具调用耗时
- 终止原因
- 错误和取消事件

## 生产要求

- 替换所有示例密钥和数据库密码。
- 按流量设置采样率。
- 对 Prompt、工具参数和凭据执行脱敏。
- 配置 Collector 不可用和批量导出失败告警。
- 在进程退出前刷新并关闭 Span Processor。

文件导出适合离线审计，但必须限制目录权限、文件保留期和敏感字段。
