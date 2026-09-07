# 模型 Provider

Harness 通过模型 Seam 接入真实模型服务。Provider 插件负责协议、鉴权、请求、流式响应组装和错误映射；Consumer 只依赖 `ModelProvider` 契约。

接入一个 Provider 至少验证：

- 普通请求和结构化响应
- 流式增量和结束事件
- 工具调用参数组装
- 鉴权、限流和协议错误
- 请求超时与取消
- 关闭时 HTTP 客户端和后台任务清理

模型地址、模型名称和凭据通过 Profile 或受控环境配置注入。不要把密钥写入源码、TOML 或日志。
