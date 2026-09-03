//! Rust-only contract fixture runner for deterministic seam behavior.
//!
//! Fixtures define a versioned, language-neutral input and expected normalized
//! outcome. This runner never loads Python or compares implementation details.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ah_contracts::agent::{
    AgentFailure, AgentLoopRuntime, AgentRequest, AgentRunState, ApplicationRuntime,
};
use ah_contracts::controller::{Controller, IntentType, Task, TaskStatus};
use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{AGENT_LOOP, FS, SESSION_MANAGER, SESSIONS, TOOLS};
use ah_contracts::session::{SessionEventKind, SessionLog, SessionManager};
use ah_contracts::tools::ToolRegistry;
use ah_contracts::workflow::{WorkflowEngine, WorkflowSpec};
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use serde_json::{Value, json};

const FIXTURE_SCHEMA_VERSION: u32 = 1;

type Fixture = Value;

fn fixture_cases(fixture: &Fixture) -> &[Value] {
    fixture["cases"].as_array().expect("fixture cases")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

fn load_fixture(name: &str) -> Fixture {
    let path = fixtures_dir().join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("load fixture {name} at {path:?}: {error}"));
    let fixture: Fixture =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse fixture {name}: {error}"));
    assert_eq!(
        fixture["schema_version"].as_u64(),
        Some(FIXTURE_SCHEMA_VERSION as u64),
        "unsupported fixture schema for {name}"
    );
    assert_eq!(
        fixture["seam"].as_str(),
        Some(name),
        "fixture seam does not match file name"
    );
    fixture
}

fn run_fixture<F>(name: &str, mut execute: F)
where
    F: FnMut(&Value) -> Value,
{
    let fixture = load_fixture(name);
    for case in fixture_cases(&fixture) {
        let actual = execute(case);
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for {name}/{}",
            case["name"]
        );
    }
}

fn root_for(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ah-rust-contract-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ))
}

fn base_plugins(root: &Path) -> Vec<DynPlugin> {
    let session_dir = root.join("sessions");
    vec![
        Arc::new(ah_plugins_mock::MockPlugin),
        Arc::new(ah_plugins_tools::ToolsPlugin),
        Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
        Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
            session_dir.join("default.jsonl"),
            session_dir,
        )),
    ]
}
fn session_kind(value: &Value) -> SessionEventKind {
    match value.as_str().expect("session event kind") {
        "user" => SessionEventKind::User,
        "assistant" => SessionEventKind::Assistant,
        "system" => SessionEventKind::System,
        other => panic!("unsupported session event kind: {other}"),
    }
}

fn session_error_class(message: &str) -> &'static str {
    if message.starts_with("invalid session line:") {
        "invalid_session_line"
    } else if message.starts_with("unsupported session log version:") {
        "unsupported_session_log_version"
    } else if message.contains("sequence is not contiguous") {
        "non_contiguous_sequence"
    } else {
        "session_error"
    }
}

#[test]
fn rust_contract_session_fixture() {
    let root = root_for("session");
    let ctx = Context::new();
    let effects = ctx
        .mount_all(base_plugins(&root))
        .expect("mount session plugins");
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .expect("session manager");

    run_fixture("session", |case| {
        let input = &case["input"];
        match case["name"].as_str().expect("case name") {
            "append_and_derive_messages" => {
                let log = manager.create("contract-messages").expect("create session");
                let mut events = Vec::new();
                for event in input["events"].as_array().expect("events") {
                    let appended = log
                        .append(
                            session_kind(&event["kind"]),
                            json!({"content": event["content"]}),
                        )
                        .expect("append session event");
                    events.push(json!({"seq": appended.seq, "kind": event["kind"]}));
                }
                let roles: Vec<&str> = log
                    .derive_messages()
                    .iter()
                    .map(|message| match message.role {
                        ah_contracts::llm::ChatRole::User => "user",
                        ah_contracts::llm::ChatRole::Assistant => "assistant",
                        _ => "other",
                    })
                    .collect();
                json!({
                    "events": events,
                    "since_after_first": log.since(0).len(),
                    "derived_roles": roles,
                })
            }
            "jsonl_roundtrip_resume" => {
                let log = manager.create("contract-resume").expect("create session");
                for event in input["events"].as_array().expect("events") {
                    log.append(SessionEventKind::User, json!({"content": event["content"]}))
                        .expect("append session event");
                }
                let path = root.join("sessions").join("contract-resume.jsonl");
                let resumed = ah_plugins_session_log::JsonlSessionLog::open(&path, ctx.clone())
                    .expect("reopen session");
                let events = resumed.events();
                let next_seq = events.last().map(|event| event.seq + 1).unwrap_or(0);
                json!({
                    "event_count_before_resume": events.len(),
                    "resumed_event_count": events.len(),
                    "next_seq": next_seq,
                })
            }
            "fork_isolated_history" => {
                let source = manager
                    .create(input["source_id"].as_str().expect("source_id"))
                    .expect("create source");
                for event in input["events"].as_array().expect("events") {
                    source
                        .append(
                            session_kind(&event["kind"]),
                            json!({"content": event["content"]}),
                        )
                        .expect("append source");
                }
                let fork = manager
                    .fork(
                        input["source_id"].as_str().expect("source_id"),
                        input["target_id"].as_str().expect("target_id"),
                    )
                    .expect("fork session");
                source
                    .append(
                        SessionEventKind::User,
                        json!({"content": input["source_after_fork"]}),
                    )
                    .expect("append source after fork");
                json!({
                    "source_event_count": source.events().len(),
                    "fork_event_count": fork.events().len(),
                    "fork_last_content": fork.events().last().expect("fork event").payload["content"],
                })
            }
            "checkpoint_persists_snapshot" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let checkpoint = input["checkpoint"].as_str().expect("checkpoint");
                let log = manager
                    .create(session_id)
                    .expect("create checkpoint session");
                for event in input["events"].as_array().expect("events") {
                    log.append(
                        session_kind(&event["kind"]),
                        json!({"content": event["content"]}),
                    )
                    .expect("append checkpoint event");
                }
                manager
                    .checkpoint(session_id, checkpoint)
                    .expect("checkpoint");
                let checkpoint_path = root
                    .join("sessions")
                    .join("checkpoints")
                    .join(format!("{checkpoint}.jsonl"));
                let snapshot =
                    ah_plugins_session_log::JsonlSessionLog::open(&checkpoint_path, ctx.clone())
                        .expect("open checkpoint snapshot");
                json!({
                    "checkpoint_event_count": snapshot.events().len(),
                    "checkpoint_exists": checkpoint_path.exists(),
                })
            }
            "restore_rolls_back_and_continues" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let checkpoint = input["checkpoint"].as_str().expect("checkpoint");
                let log = manager.create(session_id).expect("create restore session");
                log.append(SessionEventKind::User, json!({"content": input["before"]}))
                    .expect("append before restore");
                manager
                    .checkpoint(session_id, checkpoint)
                    .expect("checkpoint restore point");
                log.append(SessionEventKind::User, json!({"content": input["after"]}))
                    .expect("append after restore point");
                let restored = manager
                    .restore(session_id, checkpoint)
                    .expect("restore session");
                let restored_events = restored.events();
                let next_seq = restored_events
                    .last()
                    .map(|event| event.seq + 1)
                    .unwrap_or(0);
                let disk = manager.open(session_id).expect("open restored session");
                json!({
                    "restored_event_count": restored_events.len(),
                    "restored_content": restored_events[0].payload["content"],
                    "next_seq": next_seq,
                    "disk_event_count": disk.events().len(),
                })
            }
            "corrupted_log_rejected" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let path = root.join("sessions").join(format!("{session_id}.jsonl"));
                std::fs::write(&path, input["content"].as_str().expect("content"))
                    .expect("write corrupt log");
                let result = manager.create(session_id);
                json!({
                    "ok": result.is_ok(),
                    "error_class": result.err().map(|error| session_error_class(&error.0)),
                })
            }
            "unsupported_log_version_rejected" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let path = root.join("sessions").join(format!("{session_id}.jsonl"));
                let line = json!({
                    "version": input["version"],
                    "event": {"seq": 0, "timestamp_ms": 0, "kind": "user", "payload": {"content": "version"}}
                });
                std::fs::write(&path, line.to_string()).expect("write versioned log");
                let result = manager.create(session_id);
                json!({
                    "ok": result.is_ok(),
                    "error_class": result.err().map(|error| session_error_class(&error.0)),
                })
            }
            "concurrent_append_is_contiguous" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let path = root.join("sessions").join(format!("{session_id}.jsonl"));
                let log = Arc::new(
                    ah_plugins_session_log::JsonlSessionLog::open(&path, ctx.clone())
                        .expect("open concurrent session"),
                );
                let workers = input["workers"].as_u64().expect("workers");
                let per_worker = input["events_per_worker"]
                    .as_u64()
                    .expect("events_per_worker");
                std::thread::scope(|scope| {
                    for worker in 0..workers {
                        let log = Arc::clone(&log);
                        scope.spawn(move || {
                            for event in 0..per_worker {
                                log.append(
                                    SessionEventKind::User,
                                    json!({"content": format!("{worker}-{event}")}),
                                )
                                .expect("concurrent append");
                            }
                        });
                    }
                });
                let events = log.events();
                let contiguous = events
                    .iter()
                    .enumerate()
                    .all(|(index, event)| event.seq == index as u64);
                let reopened = ah_plugins_session_log::JsonlSessionLog::open(&path, ctx.clone())
                    .expect("reopen concurrent session");
                json!({
                    "event_count": events.len(),
                    "seq_first": events.first().map(|event| event.seq).unwrap_or(0),
                    "seq_last": events.last().map(|event| event.seq).unwrap_or(0),
                    "persisted_event_count": reopened.events().len(),
                    "seq_contiguous": contiguous,
                })
            }
            "restore_is_idempotent" => {
                let session_id = input["session_id"].as_str().expect("session_id");
                let checkpoint = input["checkpoint"].as_str().expect("checkpoint");
                let log = manager
                    .create(session_id)
                    .expect("create idempotent session");
                log.append(SessionEventKind::User, json!({"content": input["content"]}))
                    .expect("append idempotent event");
                manager
                    .checkpoint(session_id, checkpoint)
                    .expect("checkpoint idempotent");
                log.append(SessionEventKind::User, json!({"content": "discard"}))
                    .expect("append after idempotent checkpoint");
                let first = manager
                    .restore(session_id, checkpoint)
                    .expect("first restore");
                let second = manager
                    .restore(session_id, checkpoint)
                    .expect("second restore");
                let disk = manager.open(session_id).expect("open idempotent session");
                let next_seq = second
                    .events()
                    .last()
                    .map(|event| event.seq + 1)
                    .unwrap_or(0);
                json!({
                    "first_restore_count": first.events().len(),
                    "second_restore_count": second.events().len(),
                    "disk_event_count": disk.events().len(),
                    "next_seq": next_seq,
                })
            }
            other => panic!("unknown session fixture case: {other}"),
        }
    });

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rust_contract_agent_loop_fixture() {
    let root = root_for("agent-loop");
    let session_dir = root.join("sessions");
    let ctx = Context::new();
    let effects = ctx
        .mount_all(vec![
            Arc::new(ah_plugins_mock::MockPlugin),
            Arc::new(ah_plugins_agent_control::AgentControlPlugin),
            Arc::new(ah_plugins_model_backup::ModelBackupPlugin::new(Vec::new())),
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                session_dir.join("default.jsonl"),
                &session_dir,
            )),
            Arc::new(ah_plugins_agent_loop::AgentLoopPlugin::default()),
        ])
        .expect("mount agent-loop plugins");
    let agent = ctx
        .service::<dyn AgentLoopRuntime>(&AGENT_LOOP)
        .expect("agent-loop");
    let sessions = ctx
        .service::<dyn SessionLog>(&SESSIONS)
        .expect("session log");
    let fixture = load_fixture("agent_loop");

    for case in fixture_cases(&fixture) {
        let before_event_count = sessions.events().len();
        let result = agent
            .run(case["input"]["text"].as_str().expect("input text"))
            .await;
        let events = sessions.events();
        let event_kinds: Vec<&str> = events[before_event_count..]
            .iter()
            .filter_map(|event| match event.kind {
                SessionEventKind::User => Some("user"),
                SessionEventKind::Assistant => Some("assistant"),
                SessionEventKind::ToolResult => Some("tool_result"),
                // Internal lifecycle/delta events are not model-visible trace items.
                _ => None,
            })
            .collect();
        let answer_contains = case["input"]["answer_contains"]
            .as_str()
            .map(|needle| {
                result
                    .answer
                    .as_deref()
                    .is_some_and(|answer| answer.contains(needle))
            })
            .unwrap_or(false);
        let actual = json!({
            "state": agent_state_name(result.state),
            "failure": result.failure.map(agent_failure_name),
            "iterations": result.iterations,
            "tool_calls": result.tool_calls,
            "answer_contains": answer_contains,
            "event_kinds": event_kinds,
        });
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for agent_loop/{}",
            case["name"]
        );
    }

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

fn agent_state_name(state: AgentRunState) -> &'static str {
    match state {
        AgentRunState::Running => "running",
        AgentRunState::Interrupted => "interrupted",
        AgentRunState::Cancelled => "cancelled",
        AgentRunState::TimedOut => "timed_out",
        AgentRunState::Completed => "completed",
        AgentRunState::Failed => "failed",
    }
}

fn agent_failure_name(failure: AgentFailure) -> &'static str {
    match failure {
        AgentFailure::InvalidInput => "invalid_input",
        AgentFailure::Interrupted => "interrupted",
        AgentFailure::Cancelled => "cancelled",
        AgentFailure::TimedOut => "timed_out",
        AgentFailure::Session => "session",
        AgentFailure::Model => "model",
        AgentFailure::Context => "context",
        AgentFailure::IterationLimit => "iteration_limit",
        AgentFailure::ToolRecoveryRequired => "tool_recovery_required",
    }
}
#[tokio::test]
async fn rust_contract_workflow_fixture() {
    let root = root_for("workflow");
    let session_dir = root.join("sessions");
    let ctx = Context::new();
    let effects = ctx
        .mount_all(vec![
            Arc::new(ah_plugins_mock::MockPlugin),
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
            Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                session_dir.join("default.jsonl"),
                &session_dir,
            )),
            Arc::new(ah_plugins_web::WebPlugin),
            Arc::new(ah_plugins_queue::QueuePlugin::new(root.join("queue"))),
            Arc::new(ah_plugins_workflow::WorkflowPlugin),
        ])
        .expect("mount workflow plugins");
    let engine = ctx
        .service::<dyn WorkflowEngine>(&ah_contracts::keys::WORKFLOW)
        .expect("workflow engine");
    let fs = ctx.service::<dyn FsProvider>(&FS).expect("filesystem");
    let fixture = load_fixture("workflow");

    for case in fixture_cases(&fixture) {
        let input = &case["input"];
        let spec: WorkflowSpec =
            serde_json::from_value(input["spec"].clone()).expect("workflow spec");
        let actual = match case["name"].as_str().expect("case name") {
            "sequential_tool_llm_trace" => {
                let output = engine
                    .run(&spec, input["input"].clone())
                    .await
                    .expect("workflow run");
                let content = output.output["content"].as_str().unwrap_or_default();
                json!({
                    "executed": output.executed,
                    "output_content_contains": content.contains(
                        input["answer_contains"].as_str().expect("answer needle")
                    ),
                    "file_content": String::from_utf8(fs.read("workflow.txt").expect("workflow file"))
                        .expect("utf8 file content"),
                })
            }
            "checkpoint_reuses_completed_nodes" => {
                let checkpoint = root.join("workflow-checkpoint.jsonl");
                let first = engine
                    .run_checkpointed(&spec, input["input"].clone(), &checkpoint)
                    .await
                    .expect("first checkpointed run");
                let second = engine
                    .run_checkpointed(&spec, input["input"].clone(), &checkpoint)
                    .await
                    .expect("resumed checkpointed run");
                json!({
                    "first_executed": first.executed,
                    "second_executed": second.executed,
                    "second_resumed": second.resumed,
                    "second_output": second.output,
                })
            }
            "cycle_rejected" => {
                let error = engine
                    .run(&spec, input["input"].clone())
                    .await
                    .expect_err("cycle must be rejected");
                json!({
                    "ok": false,
                    "error_class": if error.0.contains("cycle") {
                        "workflow_cycle"
                    } else {
                        "workflow_error"
                    },
                })
            }
            other => panic!("unknown workflow fixture case: {other}"),
        };
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for workflow/{}",
            case["name"]
        );
    }

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rust_contract_application_fixture() {
    let root = root_for("application");
    std::fs::create_dir_all(&root).expect("application root");
    let session_path = root.join("default.jsonl");
    let session_dir = root.join("sessions");
    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/dev.toml");
    let (ctx, effects) = ah_app::boot(
        profile,
        &root,
        &session_path,
        &session_dir,
        &root.join("memory"),
        &root.join("retrieval"),
        &root.join("telemetry"),
    )
    .expect("dev application boot");
    let application = ctx
        .service::<dyn ApplicationRuntime>(&ah_contracts::keys::APPLICATION)
        .expect("application runtime");
    let fixture = load_fixture("application");

    for case in fixture_cases(&fixture) {
        let input = &case["input"];
        let result = application
            .invoke(AgentRequest {
                session_id: input["session_id"].as_str().unwrap_or_default().to_string(),
                input: input["input"].as_str().unwrap_or_default().to_string(),
                workflow: input.get("workflow").cloned(),
                timeout_ms: input["timeout_ms"].as_u64(),
                restore_checkpoint: None,
                command: None,
                user_id: None,
                model: None,
                temperature: None,
            })
            .await;
        let actual = match result {
            Ok(result) => {
                if case["name"] == "dev_boot_workflow_invoke" {
                    json!({
                        "state": agent_state_name(result.state),
                        "failure": result.failure.map(agent_failure_name),
                        "session_id": result.session_id,
                        "answer": result.answer.expect("workflow answer"),
                    })
                } else {
                    let needle = input["answer_contains"].as_str().expect("answer needle");
                    json!({
                        "state": agent_state_name(result.state),
                        "failure": result.failure.map(agent_failure_name),
                        "session_id": result.session_id,
                        "answer_contains": result.answer.as_deref().is_some_and(|answer| answer.contains(needle)),
                    })
                }
            }
            Err(error) => json!({
                "ok": false,
                "error_class": if error.0.contains("session_id and input") {
                    "invalid_request"
                } else {
                    "application_error"
                },
            }),
        };
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for application/{}",
            case["name"]
        );
    }

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn rust_contract_tools_fixture() {
    let root = root_for("tools");
    let ctx = Context::new();
    let effects = ctx
        .mount_all(base_plugins(&root))
        .expect("mount tool plugins");
    let fs = ctx.service::<dyn FsProvider>(&FS).expect("filesystem");
    let registry = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .expect("tool registry");

    let fixture = load_fixture("tools");
    for case in fixture_cases(&fixture) {
        let actual = match case["name"].as_str().expect("case name") {
            "invoke_real_read_file" => {
                let input = &case["input"];
                fs.write(
                    input["file"].as_str().expect("file"),
                    input["content"].as_str().expect("content").as_bytes(),
                )
                .expect("write tool probe");
                let output = registry
                    .invoke(
                        input["tool"].as_str().expect("tool"),
                        json!({"path": input["file"]}),
                    )
                    .await
                    .expect("read_file invocation");
                json!({
                    "ok": true,
                    "tool": input["tool"],
                    "contains_expected": output.to_string().contains(
                        input["content"].as_str().expect("content"),
                    ),
                })
            }
            "unknown_tool_rejected" => {
                let result = registry
                    .invoke(
                        case["input"]["tool"].as_str().expect("tool"),
                        case["input"]["arguments"].clone(),
                    )
                    .await;
                json!({
                    "ok": result.is_ok(),
                    "error_class": result.err().map(|error| {
                        if error.0.starts_with("tool not found:") {
                            "tool_not_found"
                        } else {
                            "tool_error"
                        }
                    }),
                })
            }
            other => panic!("unknown tools fixture case: {other}"),
        };
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for tools/{}",
            case["name"]
        );
    }

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rust_contract_controller_fixture() {
    let fixture = load_fixture("controller");
    assert_eq!(
        fixture_cases(&fixture).len(),
        3,
        "controller fixture coverage changed"
    );

    for case in fixture_cases(&fixture) {
        let actual = match case["name"].as_str().expect("case name") {
            "task_lifecycle_trace" => {
                let controller = ah_plugins_controller::LocalController::new();
                for task in case["input"]["tasks"].as_array().expect("tasks") {
                    let created = Task::submitted(
                        task["session_id"].as_str().expect("session_id"),
                        task["id"].as_str().expect("id"),
                        task["task_type"].as_str().expect("task_type"),
                        task["description"].as_str().expect("description"),
                        task["priority"].as_i64().expect("priority") as i32,
                    );
                    controller.create_task(created).expect("create task");
                }
                for link in case["input"]["parent_links"]
                    .as_array()
                    .expect("parent_links")
                {
                    controller
                        .link_parent(
                            link["child"].as_str().expect("child"),
                            link["parent"].as_str().expect("parent"),
                        )
                        .expect("link parent");
                }
                for transition in case["input"]["transitions"]
                    .as_array()
                    .expect("transitions")
                {
                    controller
                        .update_status(
                            transition["id"].as_str().expect("id"),
                            parse_status(transition["to"].as_str().expect("status")),
                        )
                        .expect("status transition");
                }
                let mut tasks: Vec<_> = case["input"]["tasks"]
                    .as_array()
                    .expect("tasks")
                    .iter()
                    .map(|task| {
                        controller
                            .get_task(task["id"].as_str().expect("id"))
                            .expect("task")
                    })
                    .collect();
                tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
                json!({
                    "tasks": tasks.iter().map(|task| json!({
                        "id": task.task_id,
                        "status": status_name(task.status),
                        "parent_task_id": task.parent_task_id,
                    })).collect::<Vec<_>>(),
                    "pending_ids": controller.pending_tasks("s1").iter().map(|task| task.task_id.clone()).collect::<Vec<_>>(),
                    "root_ids": controller.filter_tasks(&ah_contracts::controller::TaskFilter {
                        is_root: true,
                        ..Default::default()
                    }).expect("root tasks").iter().map(|task| task.task_id.clone()).collect::<Vec<_>>(),
                })
            }
            "invalid_transition_rejected" => {
                let controller = ah_plugins_controller::LocalController::new();
                controller
                    .create_task(Task::submitted("s1", "illegal", "agent", "illegal", 1))
                    .expect("create task");
                let error = controller
                    .update_status("illegal", TaskStatus::Completed)
                    .expect_err("illegal transition must fail");
                json!({"ok": false, "error_class": if error.0.contains("illegal transition") {
                    "illegal_transition"
                } else {
                    "controller_error"
                }})
            }
            "intent_keywords" => {
                let queries = case["input"]["queries"].as_array().expect("queries");
                json!({
                    "intents": queries.iter().map(|query| {
                        let intent = ah_plugins_controller::LocalController::new()
                            .recognize_intent(query["text"].as_str().expect("query"));
                        json!({
                            "type": intent_type_name(intent.intent_type),
                            "has_task_text": intent.task_text.is_some(),
                            "confidence": intent.confidence,
                        })
                    }).collect::<Vec<_>>(),
                })
            }
            other => panic!("unknown controller fixture case: {other}"),
        };
        assert_eq!(
            actual, case["expect"],
            "Rust contract mismatch for controller/{}",
            case["name"]
        );
    }
}

fn parse_status(value: &str) -> TaskStatus {
    match value {
        "working" => TaskStatus::Working,
        "paused" => TaskStatus::Paused,
        "submitted" => TaskStatus::Submitted,
        "completed" => TaskStatus::Completed,
        "failed" => TaskStatus::Failed,
        "canceled" => TaskStatus::Canceled,
        other => panic!("unknown task status: {other}"),
    }
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Submitted => "submitted",
        TaskStatus::Working => "working",
        TaskStatus::Paused => "paused",
        TaskStatus::InputRequired => "input_required",
        TaskStatus::Completed => "completed",
        TaskStatus::Canceled => "canceled",
        TaskStatus::Failed => "failed",
        TaskStatus::Waiting => "waiting",
        TaskStatus::Unknown => "unknown",
    }
}

fn intent_type_name(intent: IntentType) -> &'static str {
    match intent {
        IntentType::CreateTask => "create_task",
        IntentType::PauseTask => "pause_task",
        IntentType::ResumeTask => "resume_task",
        IntentType::RetryTask => "retry_task",
        IntentType::ContinueTask => "continue_task",
        IntentType::SupplementTask => "supplement_task",
        IntentType::CancelTask => "cancel_task",
        IntentType::ModifyTask => "modify_task",
        IntentType::SwitchTask => "switch_task",
        IntentType::UnknownTask => "unknown_task",
    }
}
