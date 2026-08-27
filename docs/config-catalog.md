# 配置目录

> 基线:`agent-harness@cc561c0`。本文登记 Profile schema 和宿主实际解析的配置。
> 完整插件选择以 `profiles/*.toml` 为准,不在文档中长期复制百余项清单。

## Profile schema

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | Profile 名称 |
| `bundles` | array | 否 | Bundle 列表 |
| `bundles[].id` | string | 是 | Bundle 标识 |
| `bundles[].plugins` | string[] | 否 | 插件名称清单;Profile 展开时去重 |
| `bundles[].config` | table | 否 | 当前由宿主按插件读取的 Bundle 配置 |

当前代码实测:

| 文件 | bundle | 声明插件数 | 关键差异 | 验证状态 |
| --- | --- | ---: | --- | --- |
| `profiles/dev.toml` | `boot` | 114 | 含 `ah-plugins-mock`,本地文件型 store/queue | 本地开发组合;不证明生产能力 |
| `profiles/prod.toml` | `real` | 112 | 无 mock,含 OpenAI、Redis store/queue | 仅 mock exclusion 已验证;完整 boot 尚未门禁 |

插件数量应通过解析 Profile 计算。新增或删除插件时禁止只修改本文数字而不验证 Profile。

## 组装方式

`ah-app::boot` 执行:

1. 读取 Profile;
2. 从 `plugin_catalog` 解析插件名;
3. 读取宿主负责的 Bundle 配置并替换参数化插件;
4. 通过 `Context::mount_all` 按 provides/inject 依赖拓扑挂载;
5. 返回 `Context` 和必须持续持有的 `Effect` 列表。

目前 catalog 是编译期 Rust 映射,不是动态库发现机制。production static composition 和 production
boot smoke 尚未形成 CI 门禁。

## 宿主解析的 Bundle 配置

### Model backup

当 Bundle 包含 `ah-plugins-model-backup` 时,`ah-app` 读取:

| 字段 | 类型 | 默认 | 校验 |
| --- | --- | --- | --- |
| `backup_providers` | string[] | `[]` | 名称列表;真实 provider catalog 组装仍不完整 |
| `retries_per_model` | integer | `0` | 必须非负 |
| `attempt_timeout_ms` | integer | 无 | 提供时必须大于 0 |

非默认 retry/timeout 会驱动宿主插入 `ah-plugins-model-backup-policy`。

### Controller snapshot

当 Bundle 包含 `ah-plugins-controller` 时,`ah-app` 读取:

| 字段 | 类型 | 默认 | 校验 |
| --- | --- | --- | --- |
| `task_snapshot_path` | string | `controller/tasks.json` | 必须非空、相对 workspace、不得包含父目录逃逸 |

Controller 使用 versioned JSON envelope;未知版本和 malformed envelope 显式失败,legacy 裸数组只作兼容读取。

## Provider 配置来源

### OpenAI-compatible

`ah-plugins-openai` 通过 credentials seam 优先解析:

- `openai.api_key`;
- `openai.base_url`;
- `openai.model`。

环境变量兜底:

- `OPENAI_API_KEY`;
- `OPENAI_BASE_URL`;
- `OPENAI_MODEL`。

缺少 key 时插件挂载显式失败。默认 base URL、model 和 timeout 以插件代码为准。

### Anthropic

使用:

- `ANTHROPIC_API_KEY`;
- `ANTHROPIC_BASE_URL`;
- `ANTHROPIC_MODEL`。

当前不通过 credentials seam。缺少 key 时挂载显式失败。

### Credentials

`ah-plugins-credentials` 默认提供环境变量只读映射。`set/remove` 显式报错。Profile 不应直接保存密钥值。

## 当前仍硬编码的部署参数

以下参数仍主要由 `ah-app::plugin_catalog` 构造,尚未完整进入 Bundle 配置:

- rails budget;
- Redis store/queue URL;
- PostgreSQL URL;
- transport AgentCard;
- MCP command/args;
- external CLI 完成标记;
- agent-loop max iterations;
- 部分 telemetry、team、RSI 和外部插件目录参数。

这些是配置治理缺口。部署地址、凭据、超时和预算应逐步移入 Profile 或 credentials seam。

## 配置纪律

1. 部署环境差异必须配置化;
2. 安全不变量和协议常量保持固定;
3. 配置错误在 boot 阶段显式失败;
4. 凭据只保存引用,不写入 Profile;
5. 新配置必须同时增加解析测试、非法值测试和本文登记;
6. Profile 插件名必须由 production static composition 测试验证 catalog 可解析。
