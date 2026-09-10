# Changelog

本文件记录面向贡献者和使用者的可观察变更。日期使用 ISO 8601；未发布内容集中在 `[Unreleased]`。历史 Python differential 工具仅用于审计，不属于运行时或默认 CI 门禁。

## [Unreleased] - 2026-09-10

### Added

- 完成 Rust-only contract fixture runner，覆盖 session、tools、controller、agent-loop、workflow 和 application。
- 完成 `ah-app` 的 dev/prod Profile 插件 catalog、静态组合检查以及 `mount_all` 失败回滚验证。
- 增加 application、controller、workflow stream、session/checkpoint 恢复、工具、rails、安全策略、团队通信、检索/记忆、evolving、RSI 和外部基础设施的 Rust 实现与聚焦回归。
- 增加 Redis、Milvus、Elasticsearch、Pulsar、openGauss PostgreSQL-wire、远程 sandbox 和 OTLP/JSON telemetry 等生产 provider 接线。
- 增加 DashScope 原生 embedding/rerank、OpenAI-compatible provider、Anthropic protocol provider 及显式凭据/env 文件加载规则。
- 增加插件依赖隔离测试、生产组合 smoke/E2E 入口和审计账本生成工具。

### Changed

- 产品实现、默认测试和 CI 统一以 Rust 为准；不提供 `openjiuwen.*` Python import 或 Python 对象模型兼容。
- agent-loop/application 使用结构化运行结果与失败分类，不再依赖错误字符串判断取消、超时和中断。
- 真实 provider 失败时显式返回错误；生产路径不静默回退到本地 mock。
- 注册统一通过可逆 `Effect` 管理；应用关闭和挂载失败会释放监听器、任务、子进程及 socket。
- session、controller snapshot 和 workflow checkpoint 使用带版本的 envelope，并保留受控 legacy 读取路径。

### Verification

- 当前 workspace 门禁记录为：`cargo fmt --all --check`、`cargo clippy --offline --workspace --all-targets -- -D warnings`、`cargo test --offline --workspace`。
- 生产组合已完成 Redis 与本地确定性 OpenAI-compatible fixture 的 invoke、controller 调度/恢复和 workflow stream 验证。
- Pulsar、Elasticsearch、remote sandbox、openGauss 和 Milvus 的本地 Docker E2E 已验证。
- Anthropic protocol smoke 已验证兼容协议路径；DashScope native live smoke 因模型权限返回 HTTP 403 且按要求豁免，仍标记为未验证。

[Unreleased]: ssh://gitcode.com/next-lyle/jiuwen_core_rs/compare/HEAD...HEAD
