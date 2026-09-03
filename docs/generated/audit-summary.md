# Generated capability audit summary

> Generated from `audit/ledger.json`; recorded at 2026-09-03; code revision `4892b66`; reference revision `aeb88cd8`.

Percentages below are status shares, not weighted capability completion.

## Overall

| Status | Count | Share |
| --- | ---: | ---: |
| done | 12 | 46.2% |
| partial | 13 | 50.0% |
| missing | 1 | 3.8% |
| excluded | 0 | 0.0% |
| **total** | **26** | **100.0%** |

## Domain status shares

| Domain | Total | Done | Partial | Missing | Excluded | Done share |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| agent | 3 | 3 | 0 | 0 | 0 | 100.0% |
| application | 1 | 0 | 1 | 0 | 0 | 0.0% |
| context | 1 | 0 | 1 | 0 | 0 | 0.0% |
| controller | 1 | 0 | 1 | 0 | 0 | 0.0% |
| evolving | 1 | 0 | 1 | 0 | 0 | 0.0% |
| external | 1 | 0 | 1 | 0 | 0 | 0.0% |
| governance | 7 | 6 | 1 | 0 | 0 | 85.7% |
| lifecycle | 1 | 1 | 0 | 0 | 0 | 100.0% |
| production | 1 | 1 | 0 | 0 | 0 | 100.0% |
| providers | 1 | 0 | 0 | 1 | 0 | 0.0% |
| rails | 1 | 0 | 1 | 0 | 0 | 0.0% |
| retrieval | 1 | 0 | 1 | 0 | 0 | 0.0% |
| rsi | 1 | 0 | 1 | 0 | 0 | 0.0% |
| session | 1 | 1 | 0 | 0 | 0 | 100.0% |
| subagents | 1 | 0 | 1 | 0 | 0 | 0.0% |
| teams | 1 | 0 | 1 | 0 | 0 | 0.0% |
| tools | 1 | 0 | 1 | 0 | 0 | 0.0% |
| workflow | 1 | 0 | 1 | 0 | 0 | 0.0% |

## Incomplete work packages

| ID | Phase | Domain | Title | Status |
| --- | --- | --- | --- | --- |
| P1-05 | P1 | application | Complete application binding | partial |
| P1-06 | P1 | controller | Complete controller behavior | partial |
| P1-07 | P1 | workflow | Workflow streaming execution | partial |
| P1-10 | P1 | governance | Versioned serialization contracts | partial |
| P2-01 | P2 | tools | Common tool parity | partial |
| P2-02 | P2 | rails | Rails and security policy | partial |
| P2-03 | P2 | context | Context engine | partial |
| P2-04 | P2 | subagents | Subagents | partial |
| P2-05 | P2 | teams | Multi-agent and messager | partial |
| P2-06 | P2 | retrieval | Production retrieval and memory backends | partial |
| P3-01 | P3 | evolving | Agent evolving LLM loop | partial |
| P3-02 | P3 | rsi | RSI orchestration | partial |
| P3-03 | P3 | external | External infrastructure | partial |
| P3-04 | P3 | providers | Vendor-specific providers | missing |

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

- Status: `partial`
- Implementation: `crates/ah-contracts/src/agent.rs`, `crates/ah-plugins-agent-loop/src/lib.rs`, `crates/ah-plugins-application/src/lib.rs`, `crates/ah-app/tests/differential.rs`, `references/application.json`
- Verification: `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo test --offline -p ah-plugins-application (18 passed)`, `streams_workflow_through_application_runtime passed`, `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo test --offline -p ah-app --test differential reference_application_controller_contracts (passed)`, `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo run --offline -p ah-app --bin ah-app -- profiles/dev.toml passed`, `memory_context_does_not_cross_user_boundaries passed`
- Production: dev ApplicationRuntime::invoke and workflow stream host path passed; production external provider verification remains separate
- Differential: Rust-only reference and contract evidence; Python runtime is not a product dependency

### P1-06 — Complete controller behavior

- Status: `partial`
- Implementation: `crates/ah-contracts/src/controller.rs`, `crates/ah-plugins-controller/src/lib.rs`, `crates/ah-plugins-controller/Cargo.toml`, `crates/ah-app/tests/differential.rs`, `references/controller.json`
- Verification: `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo test --offline -p ah-plugins-controller (28 passed)`, `pending_scheduler_runs_sessions_concurrently passed through dyn Controller::run_pending`, `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo test --offline -p ah-app --test differential reference_application_controller_contracts (passed)`, `loads_version_two_snapshot_with_v1_task_shape passed`
- Production: dev controller path mounted and exercised through ah-app smoke; external production verification remains separate
- Differential: Rust-only reference and contract evidence; Python controller is historical specification only

### P1-07 — Workflow streaming execution

- Status: `partial`
- Implementation: `crates/ah-contracts/src/workflow.rs`, `crates/ah-plugins-workflow/src/lib.rs`, `crates/ah-plugins-stream/src/lib.rs`
- Verification: `CARGO_TARGET_DIR=/tmp/ah-controller-target cargo test --offline -p ah-plugins-workflow (19 passed)`, `checkpointed_stream_rejects_corrupt_checkpoint_records passed`, `checkpointed_stream_reuses_nodes_and_emits_resume_chunks passed`, `streams_workflow_through_application_runtime passed`
- Production: dev workflow streaming and checkpoint smoke passed; full external production stream pending
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

- Status: `partial`
- Implementation: `crates/ah-contracts/src/session.rs`, `crates/ah-plugins-session-log/src/lib.rs`, `crates/ah-plugins-controller/src/lib.rs`, `crates/ah-plugins-workflow/src/lib.rs`
- Verification: `session version and corruption tests`, `loads_version_two_snapshot_with_v1_task_shape passed`, `workflow checkpoint version header and legacy record loading passed`, `checkpointed_stream_rejects_corrupt_checkpoint_records passed`, `unknown snapshot/checkpoint versions remain explicitly rejected`
- Production: workflow/controller/external migration policy incomplete
- Differential: Rust-only migration tests; no Python dependency

### P2-01 — Common tool parity

- Status: `partial`
- Implementation: `crates/ah-plugins-sysop/src/lib.rs`, `crates/ah-plugins-common-tools/src/lib.rs`
- Verification: `tool contract and focused plugin tests`
- Production: dev tool smoke passed
- Differential: full tool differential pending

### P2-02 — Rails and security policy

- Status: `partial`
- Implementation: `crates/ah-plugins-rails/src/lib.rs`, `crates/ah-plugins-security/src/lib.rs`
- Verification: `deterministic rail and security tests`
- Production: dangerous command blocking verified in dev smoke
- Differential: LLM rails differential pending

### P2-03 — Context engine

- Status: `partial`
- Implementation: `crates/ah-plugins-context/src/lib.rs`, `crates/ah-plugins-context-evolver/src/lib.rs`
- Verification: `context focused tests`
- Production: not verified
- Differential: round/dialogue parity pending

### P2-04 — Subagents

- Status: `partial`
- Implementation: `crates/ah-plugins-subagent/src/lib.rs`, `crates/ah-plugins-subagents/src/lib.rs`
- Verification: `subagent focused tests`
- Production: not verified
- Differential: Python builder and E2E parity pending

### P2-05 — Multi-agent and messager

- Status: `partial`
- Implementation: `crates/ah-contracts/src/messager.rs`, `crates/ah-plugins-teams/src/lib.rs`
- Verification: `in-process messager and team lifecycle tests`
- Production: cross-process transport pending
- Differential: 2 seams matched; cross-instance case known divergence

### P2-06 — Production retrieval and memory backends

- Status: `partial`
- Implementation: `crates/ah-plugins-retrieval/src/lib.rs`, `crates/ah-plugins-memory/src/lib.rs`
- Verification: `local retrieval and memory tests`
- Production: external embedding/vector/memory E2E pending
- Differential: not verified

### P3-01 — Agent evolving LLM loop

- Status: `partial`
- Implementation: `crates/ah-plugins-evolving/src/lib.rs`
- Verification: `deterministic evolution tests`
- Production: LLM-driven loop pending
- Differential: not verified

### P3-02 — RSI orchestration

- Status: `partial`
- Implementation: `crates/ah-plugins-rsi/src/lib.rs`, `crates/ah-plugins-rsi-single-harness/src/lib.rs`
- Verification: `deterministic RSI tests`
- Production: full external orchestration pending
- Differential: not verified

### P3-03 — External infrastructure

- Status: `partial`
- Implementation: `crates/ah-plugins-store/src/lib.rs`, `crates/ah-plugins-queue/src/lib.rs`, `crates/ah-plugins-checkpointer/src/lib.rs`
- Verification: `local protocol and configuration tests`
- Production: Redis/Pulsar/ES/GaussDB/Milvus/remote sandbox/OTel verification pending
- Differential: not verified

### P3-04 — Vendor-specific providers

- Status: `missing`
- Implementation: none
- Verification: none
- Production: not implemented
- Differential: not applicable

