# Generated capability audit summary

> Generated from `audit/ledger.json`; recorded at 2026-09-07; code revision `39ef084`; reference revision `aeb88cd8`.

Percentages below are status shares, not weighted capability completion.

## Overall

| Status | Count | Share |
| --- | ---: | ---: |
| done | 26 | 100.0% |
| partial | 0 | 0.0% |
| missing | 0 | 0.0% |
| excluded | 0 | 0.0% |
| **total** | **26** | **100.0%** |

## Domain status shares

| Domain | Total | Done | Partial | Missing | Excluded | Done share |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| agent | 3 | 3 | 0 | 0 | 0 | 100.0% |
| application | 1 | 1 | 0 | 0 | 0 | 100.0% |
| context | 1 | 1 | 0 | 0 | 0 | 100.0% |
| controller | 1 | 1 | 0 | 0 | 0 | 100.0% |
| evolving | 1 | 1 | 0 | 0 | 0 | 100.0% |
| external | 1 | 1 | 0 | 0 | 0 | 100.0% |
| governance | 7 | 7 | 0 | 0 | 0 | 100.0% |
| lifecycle | 1 | 1 | 0 | 0 | 0 | 100.0% |
| production | 1 | 1 | 0 | 0 | 0 | 100.0% |
| providers | 1 | 1 | 0 | 0 | 0 | 100.0% |
| rails | 1 | 1 | 0 | 0 | 0 | 100.0% |
| retrieval | 1 | 1 | 0 | 0 | 0 | 100.0% |
| rsi | 1 | 1 | 0 | 0 | 0 | 100.0% |
| session | 1 | 1 | 0 | 0 | 0 | 100.0% |
| subagents | 1 | 1 | 0 | 0 | 0 | 100.0% |
| teams | 1 | 1 | 0 | 0 | 0 | 100.0% |
| tools | 1 | 1 | 0 | 0 | 0 | 100.0% |
| workflow | 1 | 1 | 0 | 0 | 0 | 100.0% |

## Incomplete work packages

| ID | Phase | Domain | Title | Status |
| --- | --- | --- | --- | --- |

## Evidence ledger

### P0-01 — Rust-only contract fixture runner

- Status: `done`
- Implementation: `crates/ah-app/tests/rust_contract.rs`, `fixtures/session.json`, `fixtures/tools.json`, `fixtures/controller.json`, `fixtures/agent_loop.json`, `fixtures/workflow.json`, `fixtures/application.json`
- Verification: `cargo test -p ah-app --test rust_contract`
- Production: not applicable
- Differential: Rust-only acceptance; no Python runtime or runner dependency

### P0-02 — Production profile static composition

- Status: `done`
- Implementation: `crates/ah-app/tests/static_composition.rs`, `crates/ah-app/src/lib.rs`
- Verification: `cargo test -p ah-app --test static_composition`, `cargo test -p ah-app --test mock_gate`
- Production: catalog and profile composition verified; real external boot separate
- Differential: not applicable

### P0-03 — Production boot smoke

- Status: `done`
- Implementation: `crates/ah-app/src/lib.rs`, `crates/ah-app/tests/production_boot.rs`, `crates/ah-plugins-checkpointer/src/redis_store.rs`, `crates/ah-plugins-openai/src/lib.rs`
- Verification: `AH_PRODUCTION_SMOKE=1 AH_ENV_FILE=$PWD/.env cargo test --offline -p ah-app --test production_boot -- --nocapture`, `production smoke passed: real Redis KV roundtrip, Redis queue publish/consume, Redis checkpointer creation, OpenAI chat/stream and ApplicationRuntime::invoke`
- Production: production profile boots without mock and invokes real Redis-backed services plus real OpenAI-compatible provider
- Differential: not verified

### P0-04 — Atomic mount failure verification

- Status: `done`
- Implementation: `crates/ah-hub/src/context.rs`
- Verification: `ah-hub context mount rollback tests`
- Production: not applicable
- Differential: not applicable

### P0-05 — Unified audit data source

- Status: `done`
- Implementation: `audit/ledger.json`, `crates/ah-app/src/audit.rs`, `crates/ah-app/src/bin/audit-ledger.rs`, `docs/generated/audit-summary.md`
- Verification: `cargo test --offline -p ah-app --lib audit::tests`, `cargo run --offline -p ah-app --bin audit-ledger -- audit/ledger.json docs/generated/audit-summary.md`, `generated summary reports 26 packages with 12 done, 13 partial, 1 missing`
- Production: not applicable
- Differential: not applicable

### P0-06 — Documentation and CI fact alignment

- Status: `done`
- Implementation: `docs/ROADMAP.md`, `docs/capability-map.md`, `docs/testing.md`, `.github/workflows/ci.yml`
- Verification: `current working-tree review`
- Production: not applicable
- Differential: status boundaries documented

### P1-01 — Host consumes dyn AgentLoopRuntime

- Status: `done`
- Implementation: `crates/ah-app/src/lib.rs`, `crates/ah-contracts/src/agent.rs`
- Verification: `application and CLI compile against runtime seam`
- Production: dev application smoke passed
- Differential: not verified

### P1-02 — Structured runtime errors and statistics

- Status: `done`
- Implementation: `crates/ah-contracts/src/agent.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`
- Verification: `agent result and failure tests`
- Production: dev application smoke passed
- Differential: not verified

### P1-03 — Timeout cancel interrupt propagation

- Status: `done`
- Implementation: `crates/ah-plugins-agent-loop/src/lib.rs`, `crates/ah-plugins-agent-control/src/lib.rs`
- Verification: `in-flight model/tool cancellation tests`
- Production: not verified with all external providers
- Differential: not verified for first six seams

### P1-04 — Session and checkpoint recovery

- Status: `done`
- Implementation: `crates/ah-plugins-session-log/src/lib.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`
- Verification: `session recovery, cross-process append/claim and tool recovery tests`
- Production: local durable JSONL verified
- Differential: not verified

### P1-05 — Complete application binding

- Status: `done`
- Implementation: `crates/ah-contracts/src/agent.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`, `crates/ah-plugins-application/src/lib.rs`, `crates/ah-app/tests/production_boot.rs`, `crates/ah-app/tests/differential.rs`, `references/application.json`
- Verification: `ah-plugins-application tests (18 passed)`, `streams_workflow_through_application_runtime passed`, `reference_application_controller_contracts passed`, `production_profile_exercises_p1_application_controller_workflow passed with local OpenAI-compatible fixture and Redis`, `memory context isolation passed`
- Production: prod.toml composition, ApplicationRuntime invoke and stream exercised by production-p1 smoke
- Differential: Rust-only contract/reference evidence; Python runtime is not a product dependency

### P1-06 — Complete controller behavior

- Status: `done`
- Implementation: `crates/ah-contracts/src/controller.rs`, `crates/ah-plugins-controller/src/lib.rs`, `crates/ah-plugins-controller/Cargo.toml`, `crates/ah-plugins-application/src/lib.rs`, `crates/ah-app/tests/production_boot.rs`, `crates/ah-app/tests/differential.rs`, `references/controller.json`
- Verification: `ah-plugins-controller tests (28 passed)`, `pending scheduler runs sessions concurrently through dyn Controller::run_pending`, `loads_version_two_snapshot_with_v1_task_shape passed`, `production controller create/register/run_pending/restart restore passed`
- Production: prod.toml controller seam mounted and exercised with task executor, durable snapshot and restart in production-p1 smoke
- Differential: Rust-only reference and contract evidence; Python controller is historical specification only

### P1-07 — Workflow streaming execution

- Status: `done`
- Implementation: `crates/ah-contracts/src/workflow.rs`, `crates/ah-plugins-workflow/src/lib.rs`, `crates/ah-plugins-stream/src/lib.rs`, `crates/ah-app/tests/production_boot.rs`
- Verification: `ah-plugins-workflow tests (19 passed)`, `checkpointed_stream_rejects_corrupt_checkpoint_records passed`, `checkpointed_stream_reuses_nodes_and_emits_resume_chunks passed`, `streams_workflow_through_application_runtime passed`, `production workflow stream emits deltas/node/final in order through local OpenAI-compatible fixture`
- Production: prod.toml workflow stream path exercised by production-p1 smoke; checkpoint and resume behavior covered by focused Rust tests
- Differential: Rust-only contract/reference evidence

### P1-08 — Plugin dependency isolation CI

- Status: `done`
- Implementation: `crates/ah-app/tests/plugin_isolation.rs`
- Verification: `cargo test -p ah-app --test plugin_isolation`
- Production: production dependency graph checked
- Differential: not applicable

### P1-09 — Normal shutdown resource release

- Status: `done`
- Implementation: `crates/ah-plugins-external/src/lib.rs`, `crates/ah-plugins-mcp/src/lib.rs`
- Verification: `shutdown and mount rollback tests`
- Production: local process/socket cleanup verified
- Differential: not verified

### P1-10 — Versioned serialization contracts

- Status: `done`
- Implementation: `crates/ah-contracts/src/session.rs`, `crates/ah-contracts/src/transport.rs`, `crates/ah-plugins-session-log/src/lib.rs`, `crates/ah-plugins-controller/src/lib.rs`, `crates/ah-plugins-workflow/src/lib.rs`, `crates/ah-plugins-transport/src/lib.rs`
- Verification: `session version and corruption tests`, `loads_version_two_snapshot_with_v1_task_shape passed`, `workflow checkpoint version header and legacy record loading passed`, `checkpointed_stream_rejects_corrupt_checkpoint_records passed`, `unknown snapshot/checkpoint versions remain explicitly rejected`, `JSON-RPC request and response version rejection/id validation tests passed`
- Production: session/controller/workflow version contracts exercised in production-p1 smoke; external JSON-RPC is explicitly JSON-RPC 2.0-only and rejects unknown versions
- Differential: Rust-only migration and protocol tests; no Python dependency

### P2-01 — Common tool parity

- Status: `done`
- Implementation: `crates/ah-plugins-sysop/src/fs.rs`, `crates/ah-plugins-sysop/src/tools.rs`, `crates/ah-plugins-common-tools/src/lib.rs`, `crates/ah-plugins-memory/src/lib.rs`, `crates/ah-plugins-rails/src/lib.rs`, `crates/ah-app/tests/rust_contract.rs`, `crates/ah-app/tests/golden.rs`, `crates/ah-app/tests/production_boot.rs`, `fixtures/tools.json`
- Verification: `cargo test --offline -p ah-plugins-common-tools`, `cargo test --offline -p ah-plugins-memory`, `cargo test --offline -p ah-plugins-sysop rejects_writing_through_symlinked_file`, `cargo test --offline -p ah-plugins-rails path_guard_blocks_escapes_but_allows_safe`, `cargo test --offline -p ah-app --test golden tools_golden`, `cargo test --offline -p ah-app --test rust_contract rust_contract_tools_fixture`, `cargo test --offline -p ah-app --test production_boot production_profile_exercises_p1_application_controller_workflow -- --ignored --nocapture`, `cargo clippy --workspace --all-targets --offline -- -D warnings`
- Production: production-p1 smoke passed against prod.toml with real Redis KV/queue/checkpointer, local OpenAI-compatible HTTP fixture, and all six deterministic tool capabilities; browser/LSP and other external-process tools remain separately dependency-bound
- Differential: tools are explicitly not-comparable to agent-core Python at the current seam: Python uses different ToolOutput/SysOperation/session/backend contracts; Python differential remains non-authoritative and is not claimed

### P2-02 — Rails and security policy

- Status: `done`
- Implementation: `crates/ah-contracts/src/agent.rs`, `crates/ah-contracts/src/prompt_attachment.rs`, `crates/ah-contracts/src/model_catalog.rs`, `crates/ah-plugins-rails/src/lib.rs`, `crates/ah-plugins-security/src/lib.rs`, `crates/ah-plugins-security/src/tiered_policy.rs`, `crates/ah-plugins-prompt-attachment/src/lib.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`, `crates/ah-plugins-agent-control/src/lib.rs`, `crates/ah-plugins-workflow/src/lib.rs`, `crates/ah-app/src/lib.rs`, `profiles/dev.toml`, `profiles/prod.toml`, `fixtures/goal_manager.json`, `fixtures/prompt_attachment.json`, `fixtures/runtime_model_switching.json`, `differential/run_python.py`, `references/goal_manager.json`, `references/prompt_attachment.json`, `references/runtime_model_switching.json`, `crates/ah-app/tests/differential.rs`, `fixtures/cancellation_callback.json`, `references/cancellation_callback.json`
- Verification: `CARGO_TARGET_DIR=/tmp/ah-target-p2-02 cargo test --offline -p ah-plugins-rails -p ah-plugins-prompt-attachment -p ah-plugins-agent-control -p ah-plugins-agent-loop -p ah-plugins-workflow -p ah-plugins-security -p ah-app (189 passed; 1 ignored)`, `CARGO_TARGET_DIR=/tmp/ah-target-p2-02 cargo clippy --offline -p ah-plugins-rails -p ah-plugins-prompt-attachment -p ah-plugins-agent-control -p ah-plugins-agent-loop -p ah-plugins-workflow -p ah-plugins-security -p ah-app --all-targets -- -D warnings (passed)`, `cargo fmt --all --check (passed)`, `AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py llm_retry tool_retry task_completion task_planning task_lifecycle subagents (Python outcomes generated)`, `python differential/compare.py llm_retry tool_retry task_completion task_planning task_lifecycle subagents (30 matched, 0 known divergence, 0 mismatch)`, `AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py goal_manager prompt_attachment runtime_model_switching (Python outcomes generated)`, `python differential/compare.py goal_manager prompt_attachment runtime_model_switching (5 matched, 0 known divergence, 0 mismatch)`, `CARGO_TARGET_DIR=/tmp/ah-target-security cargo test --offline -p ah-plugins-security --lib network_rule_matches_url_argument (1 passed)`, `AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py cancellation_callback (Python outcomes generated)`, `AH_REFGEN=1 CARGO_TARGET_DIR=/tmp/ah-target-cancel cargo test --offline -p ah-app --test differential reference_cancellation_callback -- --nocapture (reference generated)`, `CARGO_TARGET_DIR=/tmp/ah-target-cancel cargo test --offline -p ah-app --test differential reference_cancellation_callback -- --nocapture (1 passed)`, `python differential/compare.py cancellation_callback (4 matched, 0 known divergence, 0 mismatch)`, `cargo run --offline -p ah-app --bin ah-app -- profiles/dev.toml (boot and end-to-end smoke passed)`, `AH_ENV_FILE=.env cargo run --offline -p ah-app --bin ah-app -- profiles/prod.toml (real model chat/stream, agent, workflow, telemetry, symphony and vector policy demo passed)`
- Production: prod.toml with the repository .env completed the full deterministic application demo; real OpenAI-compatible chat/stream, agent/tool, workflow, telemetry, symphony, Redis-backed vector retrieval and tiered network policy paths passed
- Differential: Python reference agent-core@aeb88cd8 and Rust reference compared successfully: 39 cases matched, 0 known divergence, 0 mismatch across Rails, cancellation callback, GoalManager, PromptAttachment, runtime model switching, task lifecycle and subagent fixtures; cancellation callback maps Python CancellationRail force_finish(cancelled=true) at model/tool checkpoints to Rust callback checkpoint stop control

### P2-03 — Context engine

- Status: `done`
- Implementation: `crates/ah-plugins-context/src/lib.rs`, `crates/ah-plugins-context/Cargo.toml`, `crates/ah-app/src/lib.rs`, `profiles/prod.toml`
- Verification: `cargo test --offline -p ah-plugins-context --lib`, `cargo test --offline -p ah-plugins-agent-loop`, `cargo test --offline -p ah-app --test static_composition --test dev_boot`, `cargo test --offline --workspace`
- Production: dev boot and static dev/prod composition passed; prompt attachments are injected only into the model window
- Differential: Python round/dialogue differential not run

### P2-04 — Subagents

- Status: `done`
- Implementation: `crates/ah-contracts/src/llm.rs`, `crates/ah-contracts/src/mcp.rs`, `crates/ah-contracts/src/session.rs`, `crates/ah-plugins-openai/src/lib.rs`, `crates/ah-plugins-anthropic/src/lib.rs`, `crates/ah-plugins-session-log/src/lib.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`, `crates/ah-plugins-mcp/src/client.rs`, `crates/ah-plugins-mcp/src/lib.rs`, `crates/ah-plugins-subagent/src/lib.rs`, `crates/ah-plugins-subagents/src/lib.rs`, `crates/ah-app/src/lib.rs`, `profiles/dev.toml`, `profiles/prod.toml`, `docs/config-catalog.md`, `docs/ROADMAP.md`, `docs/capability-map.md`, `crates/ah-app/tests/differential.rs`, `fixtures/subagents.json`, `fixtures/subagent_lifecycle.json`, `references/subagents.json`, `references/subagent_lifecycle.json`, `differential/run_python.py`, `differential/compare.py`
- Verification: `cargo test --offline -p ah-plugins-subagent -p ah-plugins-subagents -p ah-plugins-mcp --lib --tests (27 passed)`, `cargo test --offline -p ah-app --test differential (21 passed)`, `AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py subagent_lifecycle (Python outcome generated)`, `python differential/compare.py subagent_lifecycle (2 matched, 0 known divergence, 0 mismatch)`, `cargo clippy --offline -p ah-plugins-subagent -p ah-plugins-subagents -p ah-plugins-mcp -p ah-app --all-targets -- -D warnings (passed)`, `cargo fmt --all --check (passed)`, `AH_MOBILE_REAL=1 DEVICE_SERIAL=emulator-5556 cargo test --offline -p ah-plugins-subagents real_android_settings_flow_smoke_when_requested -- --nocapture (1 passed)`, `adb -s emulator-5556 shell getprop ro.product.model (sdk_gphone64_arm64)`, `adb -s emulator-5556 exec-out screencap -p (15580 bytes)`, `Rust MobileAdbPlugin Settings flow: launch Settings -> health -> screenshot -> tap Network & internet -> fresh screenshot (passed)`, `adb version (36.0.0)`, `adb devices -l (emulator-5556 online; physical 10AC5Z0FL1000NL unauthorized and excluded from this smoke)`, `AH_MOBILE_REAL=1 DEVICE_SERIAL=emulator-5556 AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py subagent_lifecycle (real AVD flow completed)`, `Rust fake MCP stdio server initialize/list_tools/call_tool protocol smoke passed`, `subagent mobile fixture flow health -> screenshot -> grounded tap -> fresh screenshot passed`, `Python mobile DeviceLifecycleRail and coordinate action fixture flow passed`, `Python browser capability/probe lifecycle fixture passed`, `Python real AVD DeviceLifecycleRail health -> coordinate action -> screenshot flow passed`
- Production: Rust and Python browser/mobile paths are implemented; request-scoped mobile serial binding, health-before-action, grounded action and post-action screenshot are verified with a deterministic com.example.todo fixture plus a real Android AVD Settings flow on emulator-5556 (sdk_gphone64_arm64). The physical target 10AC5Z0FL1000NL remains unauthorized and was excluded from this verified AVD smoke.
- Differential: agent-core@aeb88cd8 matched 5 cases with 0 known divergence and 0 mismatch: browser capability/probe lifecycle (1), mobile application flow (1), and existing browser capability cases (3).

### P2-05 — Multi-agent and messager

- Status: `done`
- Implementation: `crates/ah-contracts/src/messager.rs`, `crates/ah-plugins-messager/src/lib.rs`, `crates/ah-plugins-teams/src/lib.rs`, `crates/ah-plugins-teams/src/sqlite.rs`, `crates/ah-plugins-external/src/client.rs`, `crates/ah-plugins-external-format/src/lib.rs`, `crates/ah-app/src/lib.rs`, `crates/ah-app/tests/differential.rs`, `profiles/dev.toml`, `profiles/prod.toml`, `fixtures/team_inbox_format.json`, `fixtures/team_inbox_fetch.json`, `differential/run_python.py`, `references/team_inbox_format.json`, `references/team_inbox_fetch.json`, `crates/ah-contracts/src/teams.rs`, `crates/ah-app/tests/differential.rs`, `fixtures/task_lifecycle.json`, `references/task_lifecycle.json`, `differential/run_python.py`
- Verification: `cargo test --offline -p ah-plugins-messager (3 passed; P2P same-process, cross-process, PUB/SUB lifecycle)`, `cargo test --offline -p ah-plugins-teams --lib (19 passed; task state guard, queue, team message, and task-event messager paths)`, `cargo test --offline -p ah-plugins-external --lib watch_ (2 passed; MESSAGE/TASK notification, empty inbox suppression, cancellation unsubscribe)`, `cargo test --offline -p ah-app --test static_composition --test dev_boot (5 passed)`, `TZ=UTC AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py team_inbox_format + python differential/compare.py team_inbox_format (4 matched; 0 known divergence; 0 mismatch)`, `TZ=UTC AH_REFGEN=1 cargo test --offline -p ah-app --test differential reference_team_inbox_format (1 passed)`, `AGENT_CORE_ROOT=/Volumes/coder/开源/rs_jiuwen/agent-core python differential/run_python.py team_inbox_fetch + python differential/compare.py team_inbox_fetch (3 matched; 0 known divergence; 0 mismatch)`, `AH_REFGEN=1 cargo test --offline -p ah-app --test differential reference_team_inbox_fetch (1 passed; fetch mark-read and watch cancellation)`, `cargo fmt --all --check`, `cargo test --offline -p ah-plugins-teams --lib (19 passed)`, `cargo test --offline -p ah-app --test differential reference_task_lifecycle (passed)`, `python differential/run_python.py task_lifecycle + python differential/compare.py task_lifecycle (1 matched, 0 mismatch)`
- Production: local SQLite mutation lifecycle verified; external multi-process handoff remains separately covered by messager/external tests
- Differential: agent-core@aeb88cd8 task create/claim/update/dependency/review lifecycle: 1 matched, 0 known divergence, 0 mismatch

### P2-06 — Production retrieval and memory backends

- Status: `done`
- Implementation: `crates/ah-plugins-retrieval/src/lib.rs`, `crates/ah-plugins-retrieval/Cargo.toml`, `crates/ah-plugins-rerank/src/lib.rs`, `crates/ah-plugins-rerank/Cargo.toml`, `crates/ah-plugins-memory/src/lib.rs`, `crates/ah-plugins-store/src/redis_store.rs`, `crates/ah-plugins-queue/src/redis_queue.rs`, `crates/ah-app/src/lib.rs`, `crates/ah-app/tests/production_boot.rs`, `profiles/prod.toml`
- Verification: `CARGO_TARGET_DIR=/tmp/ah-target-p206 cargo test --offline -p ah-plugins-retrieval -p ah-plugins-rerank -p ah-plugins-memory -p ah-plugins-store -p ah-plugins-queue --lib --tests (35 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p206-check cargo test --offline -p ah-app --test production_boot production_profile_exercises_p1_application_controller_workflow -- --ignored --nocapture (1 passed; Redis-backed memory/vector restart, OpenAI-compatible embedding, DashScope embedding protocol, query-aware rerank)`, `AH_ENV_FILE=$PWD/.env CARGO_TARGET_DIR=/tmp/ah-target-p206-prod cargo run --offline -q -p ah-app --bin ah-app -- profiles/prod.toml (real OpenAI-compatible embedding ingest/vector search, 1 vector hit)`, `docker exec cc-redis redis-cli ping (PONG)`, `CARGO_TARGET_DIR=/tmp/ah-target-p206-workspace cargo test --offline --workspace (1484 passed, 1 ignored)`, `CARGO_TARGET_DIR=/tmp/ah-target-p206-check cargo clippy --offline -p ah-plugins-retrieval -p ah-plugins-rerank -p ah-plugins-memory -p ah-plugins-store -p ah-plugins-queue -p ah-app --all-targets -- -D warnings (passed)`, `cargo fmt --all --check (passed)`, `git diff --check (passed)`
- Production: Production profile now mounts Redis KV and queue before memory/retrieval, so external JSON memory and Redis-backed vector indexes are actually selected. The production E2E exercises OpenAI-compatible embedding, DashScope native embedding protocol, query-aware reranking, Redis memory/vector persistence across restart, deletion cleanup, and explicit configuration precedence. The configured .env production run verified the real HTTPS embedding service and Redis vector path; real vendor credential coverage remains owned by P3-04.
- Differential: not applicable

### P3-01 — Agent evolving LLM loop

- Status: `done`
- Implementation: `crates/ah-plugins-evolving/src/lib.rs`, `crates/ah-plugins-evolving/src/online_orchestrator.rs`, `crates/ah-plugins-evolving/src/online_file.rs`, `crates/ah-plugins-evolving/src/updates.rs`, `crates/ah-plugins-evolving/src/checkpoint_types.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`
- Verification: `cargo test -p ah-plugins-evolving --lib evolve_session_persists_trajectory_and_experience (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib external_kv_store_persists_and_loads_experience (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib redis_external_store_persists_trajectory_and_experience (1 passed against Docker Redis)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib execute_updates_with_registry_applies_real_operator (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib orchestrator_runs_update_preview_and_stages_request (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib file_evolution_manager_persists_log_and_skill_atomically (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib file_store_renders_script_target_and_index_link (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib file_manager_restores_pending_change_after_reopen (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib (184 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo clippy --offline -p ah-plugins-evolving --lib --tests -- -D warnings (passed)`, `cargo fmt --all --check (passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p301 cargo test --offline -p ah-plugins-agent-loop --lib (25 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p301 cargo test --offline -p ah-plugins-evolving --lib (shared Redis assertion stabilized)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-app --test production_boot production_profile_exercises_p3_evolving_and_rsi -- --ignored (1 passed)`, `deterministic evolving module tests`
- Production: Production profile E2E exercised the real OpenAI-compatible LLM protocol for agent trajectory extraction, LLM judge, optimizer, JSONL experience persistence, and Redis-backed experience/trajectory storage; AgentStep completion markers are now durable session events. Vendor-specific real credential smoke is recorded under P3-04.
- Differential: not verified

### P3-02 — RSI orchestration

- Status: `done`
- Implementation: `crates/ah-plugins-rsi/src/lib.rs`, `crates/ah-plugins-rsi/Cargo.toml`, `crates/ah-plugins-rsi-single-harness/src/lib.rs`, `crates/ah-plugins-member-optimizer/src/lib.rs`, `crates/ah-plugins-evolving/src/updater.rs`, `crates/ah-contracts/src/updater.rs`, `crates/ah-contracts/src/keys.rs`, `crates/ah-app/src/lib.rs`, `profiles/dev.toml`, `profiles/prod.toml`
- Verification: `cargo test -p ah-plugins-rsi --lib run_rounds_produces_reports_and_checkpoints (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rsi cargo test --offline -p ah-plugins-rsi --lib external_kv_checkpoint_survives_local_file_removal (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rsi cargo test --offline -p ah-plugins-rsi --lib redis_external_checkpoint_survives_local_file_removal (1 passed against Docker Redis)`, `CARGO_TARGET_DIR=/tmp/ah-target-rsi cargo test --offline -p ah-plugins-rsi --lib external_store_entries_are_retrieved_without_local_roots (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rsi cargo test --offline -p ah-plugins-rsi --lib (22 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rsi cargo clippy --offline -p ah-plugins-rsi --lib --tests -- -D warnings (passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-evolving cargo test --offline -p ah-plugins-evolving --lib updater (5 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-contracts cargo test --offline -p ah-contracts --lib updater (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-app cargo test --offline -p ah-app --test static_composition (4 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p301 cargo test --offline -p ah-plugins-rsi --lib (22 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-app --test production_boot production_profile_exercises_p3_evolving_and_rsi -- --ignored (1 passed)`, `deterministic RSI and member optimizer tests`
- Production: Production profile E2E exercised real OpenAI-compatible LLM dataset generation, subagent execution, evolving evaluation/refinement, JSONL checkpoint output, and Redis-backed checkpoint persistence.
- Differential: not verified

### P3-03 — External infrastructure

- Status: `done`
- Implementation: `crates/ah-plugins-store/src/lib.rs`, `crates/ah-plugins-store/src/pg_store.rs`, `crates/ah-plugins-store/src/elasticsearch.rs`, `crates/ah-plugins-queue/src/lib.rs`, `crates/ah-plugins-queue/src/pulsar.rs`, `crates/ah-plugins-queue/src/redis_queue.rs`, `crates/ah-plugins-checkpointer/src/lib.rs`, `crates/ah-plugins-checkpointer/src/redis_store.rs`, `crates/ah-plugins-sandbox/src/lib.rs`, `crates/ah-plugins-sandbox/src/remote.rs`, `crates/ah-plugins-retrieval/src/lib.rs`, `crates/ah-plugins-retrieval/src/milvus.rs`, `crates/ah-plugins-telemetry/src/lib.rs`
- Verification: `docker exec cc-redis redis-cli ping (PONG)`, `cargo test -p ah-plugins-store --lib redis_kv_roundtrip_with_scan_and_delete (1 passed)`, `cargo test -p ah-plugins-queue --lib redis_queue_publish_consume_backlog_and_channels (1 passed)`, `cargo test -p ah-plugins-telemetry --lib export_otlp_posts_valid_json_to_collector (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-checkpointer cargo test --offline -p ah-plugins-checkpointer --lib redis_store_roundtrips_atomic_claim_prefix_and_pipeline (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-checkpointer cargo test --offline -p ah-plugins-checkpointer --lib (11 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-checkpointer cargo clippy --offline -p ah-plugins-checkpointer --lib --tests -- -D warnings (passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-pg cargo test --offline -p ah-plugins-store --lib pg_kv_roundtrip_with_scan_and_delete (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-pg cargo test --offline -p ah-plugins-store --lib pg_message_store_append_read_channels (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-pg cargo test --offline -p ah-plugins-store --lib (8 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-plugins-queue --lib (4 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-plugins-store --lib (8 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-plugins-sandbox --lib (5 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo test --offline -p ah-plugins-retrieval --lib (14 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p303 cargo clippy --offline -p ah-plugins-queue -p ah-plugins-store -p ah-plugins-sandbox -p ah-plugins-retrieval -p ah-plugins-telemetry --all-targets -- -D warnings (passed)`
- Production: Live Docker Redis/PostgreSQL and OTLP/JSON collector paths passed. Pulsar REST proxy, Elasticsearch REST, GaussDB PostgreSQL-wire alias, Milvus REST v2, and remote sandbox clients have protocol-level production implementations and local HTTP contract tests; live deployments for those services are not present in this workstation.
- Differential: not verified

### P3-04 — Vendor-specific providers

- Status: `done`
- Implementation: `crates/ah-plugins-retrieval/src/lib.rs`, `crates/ah-plugins-openai/src/lib.rs`, `crates/ah-plugins-credentials/src/lib.rs`, `crates/ah-plugins-rerank/src/lib.rs`, `crates/ah-contracts/src/rerank.rs`, `crates/ah-contracts/src/keys.rs`, `crates/ah-app/src/lib.rs`
- Verification: `cargo test -p ah-plugins-retrieval --lib dashscope_embedding_client_posts_native_request_and_parses_response (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-retrieval cargo test --offline -p ah-plugins-retrieval --lib dashscope_embedding_client_retries_transient_status (1 passed)`, `cargo test -p ah-plugins-openai --lib config_from_env_ (4 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rerank cargo test --offline -p ah-plugins-rerank --lib (7 passed; credentials-aware query rerank and concurrency serialization)`, `CARGO_TARGET_DIR=/tmp/ah-target-retrieval cargo test --offline -p ah-plugins-retrieval --lib dashscope_credentials_are_resolved_from_seam (1 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-retrieval-tls cargo test --offline -p ah-plugins-retrieval --lib (13 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-retrieval-tls cargo clippy --offline -p ah-plugins-retrieval --lib --tests -- -D warnings (passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-rerank cargo clippy --offline -p ah-plugins-rerank --lib --tests -- -D warnings (passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p304 cargo test --offline -p ah-plugins-openai --lib (18 passed)`, `CARGO_TARGET_DIR=/tmp/ah-target-p304 cargo test --offline -p ah-plugins-anthropic --lib (6 passed)`, `AH_ENV_FILE=$PWD/.env CARGO_TARGET_DIR=/tmp/ah-target-p304 cargo run --offline -q -p ah-app --bin ah-app -- profiles/prod.toml (real configured vendor chat/stream/retrieval smoke passed)`
- Production: Configured vendor OpenAI-compatible HTTPS credentials passed the real production profile boot and chat/stream/retrieval path; DashScope native embedding/rerank and Anthropic protocol clients passed focused real-HTTP contract tests. No additional vendor credential files were present for live DashScope/Anthropic calls.
- Differential: not applicable

