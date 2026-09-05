# AGENTS.md

agent-harness 是插件化 agent harness(参考 DeepSeek Harness / Cordis 理念),
目标是完整实现 openJiuwen agent-core(Python)的全部功能。

在仓库中生成/修改任何代码之前,请阅读:

必读:
1. [docs/README.md](docs/README.md) — 文档集索引与阅读顺序(全量清单)
2. [docs/architecture.md](docs/architecture.md) — 架构约束(硬性规则)
3. [docs/hub-primer.md](docs/hub-primer.md) — 内核语义入门(注册表/事件/Effect/Plugin/Profile)
4. [docs/capability-map.md](docs/capability-map.md) — 能力 ↔ seam ↔ 插件 ↔ 工作包映射
5. [docs/agent-guide.md](docs/agent-guide.md) — 生成参考(任务配方与验证命令)

按需参考:
- [docs/development.md](docs/development.md) — 开发流程与 CI 门禁
- [docs/testing.md](docs/testing.md) — 测试与对等验证(契约 fixtures、差分契约、覆盖率)
- [docs/usage.md](docs/usage.md) — 用户文档(插件编写、seam 定义、事件、profile)
- [docs/glossary.md](docs/glossary.md) — 术语表(全文档集统一术语)
- [docs/event-catalog.md](docs/event-catalog.md) — 事件目录(事件 × 模式 × 生产者 × 消费者)
- [docs/config-catalog.md](docs/config-catalog.md) — 配置目录(profile 与插件配置字段)
- [docs/module-graph.md](docs/module-graph.md) — 模块依赖图(crate 布局与新增规则)

硬性规则摘要:契约零实现;插件只依赖 ah-hub + ah-contracts;注册必可逆(Effect);
mock 只进 ah-plugins-mock;生产路径禁止 todo!/unimplemented!/静默 fallback;
本地/mock 测试通过 ≠ 完成。
敏感配置硬性规则:运行时只能使用 env 文件声明的配置;env 文件覆盖同名进程变量,缺失 env 文件必须失败;禁止读取/猜测未声明的全局 provider 配置。
修改 `.env`、`.env.*` 或其他含凭据文件前必须保留原内容,禁止整体覆盖、输出或提交密钥;详细加载规则见 [docs/development.md](docs/development.md)。
