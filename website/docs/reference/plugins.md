# 内置插件目录

当前主 Workspace 共包含 **109 个 ah-plugins-* 插件 crate**。位于 `example/` 下的示例插件不计入主项目目录；本页由 scripts/generate-plugin-catalog.mjs 根据 Cargo metadata 生成。

示例插件另见 [CLI 开发工具示例](/guide/cli-example)：`ah-plugins-cli`。

> 主项目插件的稳定名称、构造方式、提供的 ServiceKey 和 Profile 组合仍以对应 crate、`crates/ah-app/src/lib.rs` 与 `profiles/*.toml` 为准。表格中的 description 来自各插件的 `Cargo.toml`。

## 如何使用目录

1. 先按场景找到插件 crate。
2. 阅读对应 `crates/<plugin>/src/lib.rs` 的公共类型和 `Plugin` 实现。
3. 查看 `provides()`、`inject()` 和 `apply()`，确认依赖与注册行为。
4. 在 `ah-app` Catalog 和 Profile 中确认可组合名称。
5. 运行该插件的测试以及 mount、resolve、invoke、unmount 集成测试。

## 运行时与基础能力

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-ability | Ability manager backed by the skill seam |
| ah-plugins-agent-control | Interrupt and lifecycle callback seams for agent runs |
| ah-plugins-agent-loop | Real ReAct agent loop driving the llm and tools seams |
| ah-plugins-application | Application routing seam for LLM and workflow agents |
| ah-plugins-context | Real context engine: token-budgeted assembly, deterministic excerpt compression with optional LLM summarization, offload to JSONL |
| ah-plugins-context-evolver | Real task memory service: JSON persistence, keyword+tag retrieval, deterministic trajectory summarization, context injection |
| ah-plugins-controller | Real controller: task lifecycle/priority/hierarchy, executor-registry scheduling with conflict handling, deterministic intent recognition |
| ah-plugins-json-parser | Real LLM-output JSON parser: fenced code extraction + whole-text fallback + streaming accumulator |
| ah-plugins-manifest | Real harness-element manifest: descriptor catalog + factory registry + kind-routed registration |
| ah-plugins-model-allocator | Real team model allocators: round-robin / by-model-name / router / intelli-router + resolve_member_model |
| ah-plugins-model-backup | 未在 Cargo.toml 声明描述。 |
| ah-plugins-model-catalog | Real OpenAI-account model catalog: /models fetch, visibility filter, priority sort, forward-compat expansion, JSON cache fallback |
| ah-plugins-pregel | Real Pregel-style superstep graph engine: state channels, conditional triggers, interruption |
| ah-plugins-prompt | Real file-backed versioned prompt registry with &#123;&#123;var&#125;&#125; rendering and explicit missing-variable errors |
| ah-plugins-prompt-attachment | Real prompt attachment core: model + semantic/sha256 hashing + xml escaping + safe id sanitization |
| ah-plugins-prompt-builder | Real section-based system prompt builder: multilingual sections, injection sanitization, diagnostic report |
| ah-plugins-prompt-builder-devtools | Real devtools prompt builders: badcase/feedback/meta-template with LLM seam |
| ah-plugins-runner | Real callback chain runner: priority-ordered execution with retry/timeout/break/rollback and metrics |
| ah-plugins-session-log | Real append-only session event log with JSONL persistence |
| ah-plugins-skill | Real skill registry (file-backed) and evaluation (subagent delegation + evolving trajectory evaluation) |
| ah-plugins-skill-creator | Real skill creator: scrape→download→filter→save→LLM-generate pipeline |
| ah-plugins-stream | Real session stream pipeline: async queue, emitter, writer manager, typed writers (OutputSchema/TraceSchema/CustomSchema) |
| ah-plugins-tag-manager | Real resource tag manager: bidirectional tag-resource index, GLOBAL semantics, update/match strategies |
| ah-plugins-tokenizer | Real deterministic tokenizer: BPE-lite char-pair merges with a built vocabulary, CJK-per-char, byte offsets for context budget |
| ah-plugins-workflow | Real workflow execution engine (Start/End/LLM/Tool/Loop/SubWorkflow/Parallel + conditional edges) |

## 工具、安全与工作区

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-code | Real code execution: write code to isolated scratch dir, run python3 subprocess with timeout, capture output, cleanup |
| ah-plugins-common-tools | Common coding-agent tools: todo (session todo list) and cron (scheduled jobs), file-backed with structured output |
| ah-plugins-lsp | Real LSP subsystem: 5-language server configs + diagnostic registry |
| ah-plugins-rails | Real rails registered as tools/pre-execute waterfall listeners |
| ah-plugins-reliability-burst | Real error-burst detectors: SlidingWindowCounter + ErrorBurstDetector + Tool/Model error-rate |
| ah-plugins-reliability-monitor | Real reliability monitor: per-member detector aggregation, remediation policy + rate-limited local auto-remediation, anomaly reporting |
| ah-plugins-reliability-tools | Real tool-call health detectors: OutputLengthDetector + RepeatToolCallDetector (stable hash) |
| ah-plugins-sandbox | Real policy-driven sandbox: sandbox.json (path prefixes, denied command patterns) consumed by a tools/pre-execute rail |
| ah-plugins-security | Real security seam: rule-based guardrails (prompt injection, secrets) + tools/pre-execute rail |
| ah-plugins-sysop | Real local system operations: confined filesystem and shell execution |
| ah-plugins-tools | Real tool registry for the tools seam (infrastructure) |
| ah-plugins-tools-metadata | Real bilingual tool metadata providers (descriptions + input param schemas) for harness built-in tools |
| ah-plugins-workspace | Real workspace manifest: workspace.json with goal lifecycle (create/update/persist) |
| ah-plugins-worktree | Real team worktree lifecycle: deterministic naming + member-state ownership matching |

## 模型与外部协议

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-a2a | Real A2A protocol adapter: openjiuwen &lt;-&gt; A2A payload transformer, agent-card adapter, client result aggregation, server URL normalization |
| ah-plugins-anthropic | Real Anthropic Messages API provider (/v1/messages, system top-level, tool_use/tool_result blocks) |
| ah-plugins-checkpointer | Real Redis checkpointer: multi-entity storage + pre/post hooks + TTL |
| ah-plugins-credentials | Real credentials seam: environment-backed credential provider (get/list; set/remove return explicit read-only errors) |
| ah-plugins-external | Real external agent runtime: third-party CLI member processes + ExternalTeamClient (inbox/fetch/read) |
| ah-plugins-external-format | Real external CLI inbound rendering: render_message/messages/task_line/task_board composing inbound-render + timefmt |
| ah-plugins-mcp | Real MCP stdio transport: subprocess client speaking newline-delimited JSON-RPC 2.0 |
| ah-plugins-messager | Cross-process ZMQ ROUTER/DEALER and PUB/SUB messager transport |
| ah-plugins-oauth | Real device-code OAuth client (Device Authorization Grant): start flow and poll for token over real HTTP |
| ah-plugins-openai | Real OpenAI-compatible HTTP model provider (chat/completions) |
| ah-plugins-queue | Real file-backed message queue: append-only JSONL per channel with persisted consume cursor (log + offset model) |
| ah-plugins-store | Real file, Redis, and PostgreSQL backends for BaseKVStore and BaseMessageStore |
| ah-plugins-telemetry | Real telemetry: in-memory span recording with JSONL file export, plus agent-step and tool-executed event listeners |
| ah-plugins-tracer-otel | Real OTel tracer extension: config/redaction/semconv/span-manager/setup decisions + attribute mapping facade |
| ah-plugins-transport | Real A2A-style agent transport: JSON-RPC 2.0 over HTTP (ureq client + minimal HTTP/1.1 server) |
| ah-plugins-web | Real HTTP client (ureq): GET with timeout, status/headers/body; TLS optional |

## 团队与协作

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-inbound-render | Real inbound team-message XML rendering: &lt;team-inbound&gt;/&lt;team-event&gt;/&lt;team-context&gt; + nested &lt;team-note&gt; + snapshot supersede |
| ah-plugins-interaction-router | Real interact-str parser: # / $name / @member prefixes -&gt; typed payloads + unknown-mention fold |
| ah-plugins-member-optimizer | Real member optimizer: attribution -&gt; plan -&gt; execute (optimizer/operator) -&gt; verify on val set -&gt; publish best refs (JSON) |
| ah-plugins-subagent | Real subagent runtime: isolated-session delegation with budget and context injection |
| ah-plugins-subagents | Typed subagents (code/research/plan/verify): kind-specific system prompts and real tool allow-lists over SubagentRuntime |
| ah-plugins-team-context | Real team session context: session_id isolation primitives |
| ah-plugins-team-context-text | Real team context message bodies: build_identity_text + build_team_info_text (bilingual) |
| ah-plugins-team-dispatch | Real team run-dispatch decision: pure 7-way truth table (create/new-session/cold-recover/resume/rejects) |
| ah-plugins-team-i18n | Real team runtime i18n: bilingual string table t(key,lang) + reply_hint_for(sender) |
| ah-plugins-team-join-descriptor | Real external team-join descriptor: JSON/env serialization + validation |
| ah-plugins-team-message | Real two-phase team message rendering: intent meta &#123;template,refs,params&#125; -&gt; single-pass &#123;&#123;ns.field&#125;&#125; substitution with whitelists |
| ah-plugins-team-monitor | Real team monitor: read-only team/member/task/message views plus a recordable teams/task event stream |
| ah-plugins-team-pool | Real team runtime pool (team_name-keyed) + InteractGate concurrency gate |
| ah-plugins-team-prompts | Real agent-team prompt assembly: team.plan mode + bridge brief/overview pure functions |
| ah-plugins-team-scheduler | Real team scheduler scan core: per-member earliest PENDING(assignee) task pick + review round decisions |
| ah-plugins-team-schema | Real agent-team schema: ssh transport validation + task graph models + infra (transport/storage) registry + blueprint validations |
| ah-plugins-team-skill | Real team skill generator: deterministic skill plan from a task, register into skill seam, validate by evaluating on the source task with repair retries |
| ah-plugins-team-skill-generator | Real team-skill generation normalization: plan/roles/workflow + SKILL.md skeleton |
| ah-plugins-team-status | Real team member/execution status state machines + status sets (departed/unreachable/settled) |
| ah-plugins-team-task-status | Real team task-status state machine: TaskStatus transition table + generic is_valid_transition |
| ah-plugins-team-verdict | Real review-vote verdict math: pass quorum ceil(threshold*n), fail when quorum unreachable |
| ah-plugins-teams | Real multi-agent team runtime: task board, dependencies, review, settle, subagent delegation |

## 评估、演进与自动化

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-agentbuilder | Real agent builder: NL task -&gt; design -&gt; workflow DSL -&gt; execution via WorkflowEngine |
| ah-plugins-autoharness | Real auto-harness orchestrator: assess/plan/implement/verify/commit/publish cycle over real git/ci/rsi/subagent seams |
| ah-plugins-bridge-compose | Real bridge avatar text compose: inbound/outbound pure-function wrappers |
| ah-plugins-data-loader | Real curriculum-balanced batch planning for dataset loading (difficulty progression + dimension round-robin) |
| ah-plugins-dataset-curator | Real replay-dataset curation from eval artifacts: case decisions, provenance, targeted seed tasks, YAML report |
| ah-plugins-evolving | Real evolving runtime: trajectory extraction from session logs, local-criteria evaluation, optimization refinements |
| ah-plugins-experience-scorer | Real experience scoring: Bayesian-smoothed effectiveness, utilization, freshness decay with version staleness, weighted score, usage-stat update |
| ah-plugins-git | Real local git operations via git subprocess (init/status/add/commit/log/diff/branch) for auto_harness |
| ah-plugins-operator | Real self-evolution operators: LLM/tool/memory/skill parameter handles with tunables, freeze, state, and update callbacks |
| ah-plugins-optimizer | Real textual-gradient optimizer: derives updates from evolution evaluations and applies them through the operator seam (freeze-aware) |
| ah-plugins-rsi | Real RSI runtime: dataset generation, case execution via subagent, evolving evaluation, prompt refinement, JSONL checkpoints |
| ah-plugins-rsi-config | Real rsi configuration models (dataclass configs + from_dict parsing helpers) |
| ah-plugins-rsi-evaluator | Real RSI trajectory tools: bounded snapshots + successful usage extraction |
| ah-plugins-sharing | Real experience sharing: local filesystem hub backend + skill-scoped ExperienceSharer (stage/dedup/flush with retry & mirror cache) |
| ah-plugins-signals | Real evolution-signal mapping: analyzer issue -&gt; trajectory_issue signals (type attribution, normalization, excerpts) |
| ah-plugins-trainer | Real self-evolution trainer: baseline -&gt; loop &#123; train forward, optimizer updates via operator seam, validation gate, improve-only checkpoint &#125; with early stop |
| ah-plugins-tune | Real tune pipeline: subagent delegation + evolving evaluation + optimization-driven prompt refinement |

## 其他插件

| 插件 crate | Cargo 描述 |
| --- | --- |
| ah-plugins-ci | Real CI gate runner: subprocess gate commands with timeout, pass/fail + output |
| ah-plugins-graph-memory | Real graph memory: entity/relation/episode knowledge graph with JSONL persistence, deterministic extraction, keyword search and neighbor traversal |
| ah-plugins-kv-cache | Real KV-cache policy hooks: sticky subagent prefetch/offload/evict signals |
| ah-plugins-memory | Persistent JSON-file or external KV-backed memory with remember/recall/forget tools |
| ah-plugins-memory-lite | Real lite memory primitives: coding memory frontmatter parse/validate/enrich/rebuild, chunk models, write results |
| ah-plugins-mock | Mock LLM stub for dev boot only (no credentials). Not a deliverable: the real LLM provider is pending. |
| ah-plugins-rerank | Hybrid and query-aware retrieval rerankers with DashScope protocol support |
| ah-plugins-resources | Real extension resources: package loading + spec resolution into parts |
| ah-plugins-retrieval | Production retrieval with local or external embedding and Redis-backed vector index |
| ah-plugins-rl | Real RL reward function (agent_rl reward part): deterministic per-case rewards with pass/fail/timeout/tool-error/iteration terms |
| ah-plugins-rl-step | Real RL training-step math: advantage (reward - value), policy ratio, clipped PPO objective, value loss, aggregated metrics (no torch) |
| ah-plugins-roster-diff | Real team roster diff + message-body rendering: joined/left/changed by member_name key, snapshot/delta bilingual text |
| ah-plugins-scheduler-render | Real scheduler message assembly: meta_* delivery payloads + fail-feedback aggregation + leader strings |
| ah-plugins-symphony | Real symphony: capability registration/fingerprint, task retrieval, orchestration planning, execution via tools or subagents |
| ah-plugins-timefmt | Real epoch-ms to human time rendering: absolute local time + relative bucket (just now/seconds/minutes/hours/days) |

## 从目录进入源码

插件 crate 的生产依赖应保持在 `ah-contracts`、`ah-hub` 和必要的外部基础库范围内；插件之间禁止直接依赖其他插件的具体类型。插件 API、生命周期和测试方法见：

- [插件简介与开发](/guide/plugin)
- [插件生命周期与测试](/guide/plugin-lifecycle)
- [Plugin API](/reference/plugin-api)
- [Catalog 与 Profile API](/reference/plugin-catalog)

