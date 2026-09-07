# 内置插件概览

`agent-harness` 的能力由 `crates/ah-plugins-*` 中的插件提供。插件名称、构造参数和当前 Profile 组合以 `crates/ah-app/src/lib.rs`、`profiles/dev.toml` 和 `profiles/prod.toml` 为准。

完整的插件 crate 清单见[内置插件全量目录](/reference/plugins)，该页面在构建前根据当前 Cargo Workspace 自动生成。

## 启动基础

| 插件 | 作用 | 常见依赖 |
| --- | --- | --- |
| `ah-plugins-credentials` | 提供只读凭据引用 | 环境变量 |
| `ah-plugins-mock` | 开发和测试用替代 Provider | 无；生产禁止 |
| `ah-plugins-tools` | ToolRegistry 和工具执行管线 | `ah-hub`、`ah-contracts` |
| `ah-plugins-common-tools` | 通用文件、读取和工作区工具 | 工作区路径 |
| `ah-plugins-sysop` | 文件系统和 Shell 操作 | 工作区路径 |
| `ah-plugins-rails` | Shell、Path、Budget 等执行护栏 | 工具事件 |
| `ah-plugins-security` | 安全检查和策略护栏 | 工具事件 |

## Agent 执行

| 插件 | 作用 |
| --- | --- |
| `ah-plugins-agent-loop` | 日志驱动的 Agent 循环 |
| `ah-plugins-agent-control` | 中断、取消和生命周期回调 |
| `ah-plugins-application` | 应用级 Agent 请求入口 |
| `ah-plugins-controller` | 任务生命周期、优先级和调度冲突 |
| `ah-plugins-runner` | 回调链、重试、超时、回滚和指标 |
| `ah-plugins-subagent` / `ah-plugins-subagents` | 子代理会话和工具白名单 |

## 状态和知识

| 插件 | 作用 |
| --- | --- |
| `ah-plugins-session-log` | JSONL append-only 会话事件日志和多会话管理 |
| `ah-plugins-checkpointer` | Checkpoint 持久化 |
| `ah-plugins-context` | 上下文预算、压缩和 offload |
| `ah-plugins-workflow` | Start、End、LLM、Tool、Loop 和条件边 |
| `ah-plugins-memory` / `ah-plugins-memory-lite` | 持久化和轻量记忆 |
| `ah-plugins-retrieval` | 知识库检索 |
| `ah-plugins-graph-memory` | 实体、关系和图检索 |

## 外部服务和协作

| 插件 | 作用 |
| --- | --- |
| `ah-plugins-openai` / `ah-plugins-anthropic` | 真实模型 Provider |
| `ah-plugins-mcp` | MCP stdio 客户端 |
| `ah-plugins-a2a` / `ah-plugins-transport` | Agent-to-Agent 和 JSON-RPC 传输 |
| `ah-plugins-store` | KV、消息和外部存储适配 |
| `ah-plugins-queue` | 消息队列 |
| `ah-plugins-teams` | 团队、消息和任务协作 |
| `ah-plugins-telemetry` / `ah-plugins-tracer-otel` | Span、日志和 OTLP 观测 |

## 开发工具和演进

`ah-plugins-agentbuilder`、`ah-plugins-prompt-builder`、`ah-plugins-skill`、`ah-plugins-evolving`、`ah-plugins-rsi`、`ah-plugins-tune`、`ah-plugins-trainer` 和相关插件提供构建、技能、评估、优化与 RSI 能力。

这些插件的公开契约、依赖和实现状态应回到对应 crate 和 `agent-harness/docs/` 目录核对。网站中的分组用于入门，不替代自动生成的能力目录。
