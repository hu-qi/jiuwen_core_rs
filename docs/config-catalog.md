# 配置目录(config-catalog.md)

> 等价 DSH 的 config-catalog:登记 profile 与插件的全部配置字段。
> 环境相关的可调参数必须是配置字段(来自 profile),不允许硬编码在插件里。

## 1. Profile(profiles/*.toml)

| 字段 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| name | string | 是 | profile 名(诊断显示) |
| bundles | array | 否 | bundle 列表,按顺序堆叠 |
| bundles[].id | string | 是 | bundle 标识 |
| bundles[].plugins | string[] | 否 | 插件名清单,去重展开 |

当前文件:profiles/dev.toml(name=dev,bundles=[{id=mock, plugins=[ah-plugins-mock]}])。

## 2. 插件注册目录(ah-app)

| 字段 | 说明 |
| --- | --- |
| 插件名(profile 引用) | 与 Plugin::name() 一致,如 ah-plugins-mock |
| 插件对象 | 目录中名称 → Arc<dyn Plugin> 映射,未来可扩展为动态注册 |

## 3. 规划:每插件配置

插件引入可调参数时,必须在 profile 中声明配置字段并在本表登记:

| 插件 | 配置字段(规划) | 说明 |
| --- | --- | --- |
| ah-plugins-openai | base_url / api_key_ref / model / timeout | LLM provider |
| ah-plugins-redis | url / ttl / namespace | 检查点与 KV |
| ah-plugins-pulsar | url / topic / subscription | 消息队列 |
| ah-plugins-elasticsearch | url / index / auth | 向量库 |
| ah-plugins-mcp | 传输类型 / 端点 | 工具传输 |

## 4. 配置纪律

1. 部署环境差异(地址、凭据、超时、预算)必须是配置字段,不是 DEFAULT_* 常量;
2. 协议常量、外部规范、安全不变量保持固定,不进配置;
3. 配置错误在加载时显式报错(fail loud),不静默跳过;
4. 凭据引用(如 api_key_ref)指向 credentials seam,不直接写入 profile。
