# 配置目录

> 当前代码 HEAD:`7404142`;完整插件选择以 `profiles/*.toml` 为准,数量由解析结果生成,不在文档中手工复制百余项清单。
> `cargo metadata --no-deps` 当前可见 112 个 workspace package;10 个新增插件已加入 workspace members,并已接入 `ah-app::plugin_catalog` 与 dev/prod Profile;完整运行验证见 `ah-app/tests/stage2_plugins.rs`。

## Profile schema

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `name` | string | 是 | Profile 名称 |
| `bundles` | array | 否 | Bundle 列表 |
| `bundles[].id` | string | 是 | Bundle 标识 |
| `bundles[].plugins` | string[] | 否 | 插件名称清单;Profile 展开时去重 |
| `bundles[].config` | table | 否 | 当前由宿主按插件读取的 Bundle 配置 |

当前代码实测与审计状态:

| 项目 | 当前事实 | 验证状态 |
| --- | --- | --- |
| `profiles/dev.toml` | dev 组合包含 mock,用于本地启动和协议冒烟 | mock gate/static composition 已有;完整 boot 当前未重新验证 |
| `profiles/prod.toml` | prod 组合不应包含 mock,依赖真实凭据和外部服务 | mock exclusion/static composition 已有;无 mock boot + invoke 尚未形成当前 HEAD 证据 |
| Profile 数量 | 不在本文手工声明;应由 Profile 解析和 Cargo catalog 生成 | 旧文档中的 114/112 数字废弃 |
| 新增插件接线 | 10 个新增插件已进入 workspace members,但尚未完整进入 `ah-app::plugin_catalog`/Profile | 不可计为 runtime 可用 |

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

`ah-app::boot` 先读取 `AH_ENV_FILE` 或自动发现的 `.env`;env 文件是 provider 配置的唯一权威来源,
并覆盖同名进程变量。`ah-plugins-openai` 通过 credentials seam 读取:

- `openai.api_key` ← `OPENAI_API_KEY`;
- `openai.base_url` ← `OPENAI_BASE_URL`;
- `openai.model` ← `OPENAI_MODEL`。

`boot()` 找不到 env 文件或缺少 key 时显式失败,不会回退到 shell/CI 中未由 env 文件声明的 provider 配置。
默认 base URL、model 和 timeout 仅在对应可选项未由 env 文件配置时使用。

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
- MCP command/args/cwd/capabilities;`PLAYWRIGHT_MCP_COMMAND`/`PLAYWRIGHT_MCP_ARGS`、`PLAYWRIGHT_RUNTIME_MCP_CWD` 或 `BROWSER_RUNTIME_MCP_CWD` 覆盖 BrowserMcpPlugin 默认 `npx -y @playwright/mcp --headless`;未显式设置 `--caps` 时启用 pdf/vision/devtools/config/network/storage/testing;
- Android ADB command/device;`ANDROID_ADB_COMMAND`/`DEVICE_SERIAL` 覆盖 MobileAdbPlugin 默认值;每次移动工具调用可用 `device_serial` 选择目标设备;
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
