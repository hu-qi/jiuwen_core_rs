# 配置与环境变量

## 模型配置

| 名称 | 用途 | 是否敏感 |
| --- | --- | --- |
| `MODEL_PROVIDER` | Provider 标识 | 否 |
| `MODEL_NAME` | 模型名称 | 否 |
| `API_BASE` | API 地址 | 通常否 |
| `API_KEY` | Provider 凭据 | 是 |
| `LLM_SSL_VERIFY` | 是否校验 TLS 证书 | 否 |

生产环境应保持 TLS 校验开启。只有明确受控的本地开发环境可以关闭，并记录原因。

## Profile

Harness Profile 声明 Bundle 和插件清单。插件名必须在 `ah-app` Catalog 中注册；挂载顺序由 `provides/inject` 依赖关系决定，不由 TOML 顺序决定。

## 凭据规则

- 不在 Markdown、源码、测试快照或 Profile 中写真实密钥。
- 不在错误和 Trace 中输出完整 Token。
- 使用运行环境的密钥管理系统注入生产凭据。
- 凭据缺失时显式失败，不能猜测全局配置或改用 Mock。

具体插件字段以 `agent-harness/docs/config-catalog.md` 和对应 Provider 文档为准。
