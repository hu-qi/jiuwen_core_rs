# AGENTS.md

agent-harness 是插件化 agent harness(参考 DeepSeek Harness / Cordis 理念),
目标是完整实现 openJiuwen agent-core(Python)的全部功能。

在仓库中生成/修改任何代码之前,请阅读:

1. [docs/architecture.md](docs/architecture.md) — 架构约束(硬性规则)
2. [docs/capability-map.md](docs/capability-map.md) — 能力 ↔ seam ↔ 插件 ↔ 工作包映射
3. [docs/agent-guide.md](docs/agent-guide.md) — 生成参考(任务配方与验证命令)
4. [docs/development.md](docs/development.md) — 开发流程与 CI 门禁

硬性规则摘要:契约零实现;插件只依赖 ah-hub + ah-contracts;注册必可逆(Effect);
mock 只进 ah-plugins-mock;生产路径禁止 todo!/unimplemented!/静默 fallback;
本地/mock 测试通过 ≠ 完成。
