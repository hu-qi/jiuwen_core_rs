//! 差分契约 references(语言中立行为快照)。
//!
//! 对同一组 fixture 输入,把真实实现的**完整可观测输出**固化为 references/{seam}.json
//! (Rust 基线;按 testing.md §3,外部 Python 参考运行将验证/覆盖这些快照)。
//! 默认运行:断言 Rust 行为与 references 一致(回归保护);
//! 设 AH_REFGEN=1 运行:重新生成 references(快照更新)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::evolving::EvolvingRuntime;
use ah_contracts::goal::{GoalAssessment, GoalAssessmentStatus, GoalRecord, GoalRuntime};
use ah_contracts::keys::{EVOLVING, RETRIEVAL, SECURITY, SESSION_MANAGER, TEAMS};
use ah_contracts::llm::{ModelProvider, ModelRequest, ModelResponse};
use ah_contracts::model_catalog::ModelProviderCatalog;
use ah_contracts::retrieval::RetrievalProvider;
use ah_contracts::security::SecurityProvider;
use ah_contracts::session::{SessionEvent, SessionEventKind, SessionManager};
use ah_contracts::teams::TeamRuntime;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use async_trait::async_trait;
use serde_json::{Value, json};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

fn references_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("references")
}

fn load_fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixtures_dir().join(format!("{name}.json")))
        .unwrap_or_else(|e| panic!("load fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse fixture {name}: {e}"))
}

/// 生成模式:写 references;否则断言与 references 一致。
fn settle(seam: &str, outcome: &Value) {
    let path = references_dir().join(format!("{seam}.json"));
    let regenerate = std::env::var("AH_REFGEN").is_ok();
    if regenerate {
        std::fs::write(&path, serde_json::to_string_pretty(outcome).unwrap() + "\n")
            .unwrap_or_else(|e| panic!("write reference {seam}: {e}"));
        println!("[refgen] wrote {path:?}");
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("missing reference {seam}: {e} (run with AH_REFGEN=1 to generate)")
    });
    let expected: Value = serde_json::from_str(&text).expect("parse reference");
    assert_eq!(
        outcome,
        &expected,
        "differential mismatch for seam {seam}:\n got: {}\nwant: {}",
        serde_json::to_string_pretty(outcome).unwrap(),
        serde_json::to_string_pretty(&expected).unwrap()
    );
}

fn root_for(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ah-diff-{tag}-{}", std::process::id()))
}

fn mount(ctx: &Context, plugins: Vec<DynPlugin>) -> Vec<ah_contracts::Effect> {
    ctx.mount_all(plugins).expect("mount")
}

/// 常用组合:mock + tools + sysop + session-log。
fn base_plugins(root: &std::path::Path) -> Vec<DynPlugin> {
    let session_dir = root.join("sessions");
    let default_path = session_dir.join("default.jsonl");
    vec![
        Arc::new(ah_plugins_mock::MockPlugin),
        Arc::new(ah_plugins_tools::ToolsPlugin),
        Arc::new(ah_plugins_sysop::SysopPlugin::new(root)),
        Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
            &default_path,
            &session_dir,
        )),
    ]
}

// ---------- session ----------

#[test]
fn reference_session_derived_messages() {
    let root = root_for("session");
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins(&root));
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .expect("manager");
    let fixture = load_fixture("session");
    let log = manager.create("d1").expect("create");
    for ev in fixture["cases"][0]["input"]["events"].as_array().unwrap() {
        let kind = match ev["kind"].as_str().unwrap() {
            "user" => SessionEventKind::User,
            "assistant" => SessionEventKind::Assistant,
            other => panic!("unknown kind {other}"),
        };
        log.append(kind, json!({ "content": ev["content"].as_str().unwrap() }))
            .expect("append");
    }
    let outcome: Value = json!({
        "derived": log.derive_messages().iter().map(|m| {
            json!({
                "role": format!("{:?}", m.role).to_lowercase(),
                "content": m.content,
            })
        }).collect::<Vec<_>>(),
    });
    settle("session", &outcome);
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- security ----------

#[test]
fn reference_security_verdict() {
    let root = root_for("security");
    let ctx = Context::new();
    let effects = mount(
        &ctx,
        vec![Arc::new(ah_plugins_security::SecurityRailPlugin)],
    );
    let security = ctx
        .service::<dyn SecurityProvider>(&SECURITY)
        .expect("security");
    let fixture = load_fixture("security");
    let outcome: Value = json!({
        "cases": fixture["cases"].as_array().unwrap().iter().map(|case| {
            let verdict = security.verdict(case["input"]["content"].as_str().unwrap());
            json!({
                "name": case["name"],
                "allow": verdict.allow,
                "decisions": verdict.decisions.iter().map(|d| json!({
                    "guardrail": d.guardrail,
                    "allow": d.allow,
                    "severity": format!("{:?}", d.severity).to_lowercase(),
                    "reason": d.reason,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    settle("security", &outcome);
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- retrieval ----------

#[test]
fn reference_retrieval_hits() {
    let root = root_for("retrieval");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_retrieval::RetrievalPlugin::new(
        root.join("kb"),
    )));
    let effects = mount(&ctx, plugins);
    let kb = ctx
        .service::<dyn RetrievalProvider>(&RETRIEVAL)
        .expect("retrieval");
    let fixture = load_fixture("retrieval");
    let input = &fixture["cases"][0]["input"];
    kb.ingest(
        input["doc_id"].as_str().unwrap(),
        input["text"].as_str().unwrap(),
        json!({}),
    )
    .expect("ingest");

    let round = |v: f64| (v * 10000.0).round() / 10000.0;
    let outcome: Value = json!({
        "bm25": kb.retrieve(input["query"].as_str().unwrap(), 2).iter().map(|h| json!({
            "doc_id": h.doc_id, "score": round(h.score), "chunk": h.chunk,
        })).collect::<Vec<_>>(),
        "vector": kb.retrieve_vector(input["query"].as_str().unwrap(), 2).iter().map(|h| json!({
            "doc_id": h.doc_id, "score": round(h.score), "chunk": h.chunk,
        })).collect::<Vec<_>>(),
    });
    settle("retrieval", &outcome);
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- teams ----------

#[tokio::test]
async fn reference_teams_lifecycle() {
    let root = root_for("teams");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_agent_control::AgentControlPlugin));
    plugins.push(Arc::new(ah_plugins_subagent::SubagentPlugin));
    plugins.push(Arc::new(ah_plugins_queue::QueuePlugin::new(
        root.join("queue"),
    )));
    plugins.push(Arc::new(ah_plugins_teams::TeamsPlugin));
    let effects = mount(&ctx, plugins);
    let teams = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");

    teams
        .create_team(
            ah_contracts::teams::TeamSpec {
                id: "t1".into(),
                name: "Alpha".into(),
            },
            vec![
                ah_contracts::teams::TeamMemberSpec {
                    id: "m1".into(),
                    name: "Alice".into(),
                    role: "dev".into(),
                },
                ah_contracts::teams::TeamMemberSpec {
                    id: "m2".into(),
                    name: "Bob".into(),
                    role: "reviewer".into(),
                },
            ],
        )
        .expect("create");
    teams
        .add_task(
            "t1",
            ah_contracts::teams::TeamTask {
                id: "a".into(),
                title: "do a".into(),
                content: "content".into(),
                status: ah_contracts::teams::TeamTaskStatus::Pending,
                dependencies: vec![],
                assignee: None,
                reviewers: vec![],
                review_votes: vec![],
                result: None,
            },
        )
        .expect("add");
    let claimed = teams.claim_task("t1", "m1").expect("claim");
    teams
        .complete_task("t1", &claimed, json!({"ok": true}))
        .expect("complete");
    teams
        .send_message("t1", "m1", Some("m2"), "hello team")
        .expect("msg");

    let tasks = teams.tasks("t1").expect("tasks");
    let messages = teams.messages("t1").expect("messages");
    let outcome: Value = json!({
        "tasks": tasks.iter().map(|t| json!({
            "id": t.id, "status": format!("{:?}", t.status).to_lowercase(),
            "assignee": t.assignee,
        })).collect::<Vec<_>>(),
        "messages": messages.iter().map(|m| json!({
            "from": m.from, "to": m.to, "content": m.content,
        })).collect::<Vec<_>>(),
    });
    settle("teams", &outcome);
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}
// ---------- task_lifecycle (Python/Rust differential) ----------

#[test]
fn reference_task_lifecycle() {
    let fixture = load_fixture("task_lifecycle");
    let root = root_for("task-lifecycle");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_agent_control::AgentControlPlugin));
    plugins.push(Arc::new(ah_plugins_subagent::SubagentPlugin));
    plugins.push(Arc::new(ah_plugins_queue::QueuePlugin::new(
        root.join("queue"),
    )));
    plugins.push(Arc::new(ah_plugins_teams::TeamsPlugin));
    let effects = mount(&ctx, plugins);
    let teams = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
    teams
        .create_team(
            ah_contracts::teams::TeamSpec {
                id: "task-team".into(),
                name: "Task Team".into(),
            },
            vec![
                ah_contracts::teams::TeamMemberSpec {
                    id: "m1".into(),
                    name: "m1".into(),
                    role: "dev".into(),
                },
                ah_contracts::teams::TeamMemberSpec {
                    id: "m2".into(),
                    name: "m2".into(),
                    role: "reviewer".into(),
                },
            ],
        )
        .expect("create team");
    let status = |task: &ah_contracts::teams::TeamTask| match task.status {
        ah_contracts::teams::TeamTaskStatus::Done => "completed",
        ah_contracts::teams::TeamTaskStatus::InProgress => "in_progress",
        ah_contracts::teams::TeamTaskStatus::InReview => "in_review",
        ah_contracts::teams::TeamTaskStatus::Pending => "pending",
        ah_contracts::teams::TeamTaskStatus::Failed => "failed",
    };
    let task_view = |task: &ah_contracts::teams::TeamTask| json!({"id": task.id, "title": task.title, "content": task.content, "status": status(task), "assignee": task.assignee});
    let mut cases = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let mut operations = Vec::new();
        for op in case["operations"].as_array().unwrap() {
            match op["op"].as_str().unwrap() {
                "create" => {
                    let result = teams.create_task(
                        "task-team",
                        op["task_id"].as_str().unwrap(),
                        op["title"].as_str().unwrap(),
                        op["content"].as_str().unwrap(),
                        vec![],
                        op["reviewers"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap().to_string())
                            .collect(),
                    );
                    operations.push(json!({"name":"create", "ok":result.is_ok(), "status":result.as_ref().ok().map(status)}));
                }
                "update" => {
                    let result = teams.update_task(
                        "task-team",
                        op["task_id"].as_str().unwrap(),
                        Some(op["title"].as_str().unwrap()),
                        Some(op["content"].as_str().unwrap()),
                    );
                    operations.push(json!({"name":"update", "ok":result.is_ok()}));
                }
                "dependency" | "dependency_cycle" => {
                    let result = teams.add_dependency(
                        "task-team",
                        op["task_id"].as_str().unwrap(),
                        op["depends_on"][0].as_str().unwrap(),
                    );
                    operations.push(json!({"name":op["op"], "ok":result.is_ok()}));
                }
                "claim" => {
                    let result = teams.claim_task("task-team", op["member"].as_str().unwrap());
                    operations.push(json!({"name":"claim", "ok":result.is_ok()}));
                }
                "complete" => {
                    let task_id = op["task_id"].as_str().unwrap();
                    let result = teams.complete_task("task-team", task_id, op["output"].clone());
                    let task = teams
                        .tasks("task-team")
                        .unwrap()
                        .into_iter()
                        .find(|task| task.id == task_id)
                        .unwrap();
                    operations.push(
                        json!({"name":"complete", "ok":result.is_ok(), "status":status(&task)}),
                    );
                }
                "review" => {
                    let task_id = op["task_id"].as_str().unwrap();
                    let vote = teams.vote_review(
                        "task-team",
                        task_id,
                        op["member"].as_str().unwrap(),
                        op["decision"].as_str().unwrap() == "pass",
                    );
                    let result =
                        vote.and_then(|_| teams.settle_review("task-team", task_id).map(|_| ()));
                    let task = teams
                        .tasks("task-team")
                        .unwrap()
                        .into_iter()
                        .find(|task| task.id == task_id)
                        .unwrap();
                    operations.push(
                        json!({"name":"review", "ok":result.is_ok(), "status":status(&task)}),
                    );
                }
                _ => panic!("unknown task operation"),
            }
        }
        let tasks = teams.tasks("task-team").unwrap();
        operations.push(
            json!({"name":"final", "tasks":tasks.iter().map(task_view).collect::<Vec<_>>() }),
        );
        cases.push(json!({"name":case["name"], "operations":operations}));
    }
    settle(
        "task_lifecycle",
        &json!({"seam":"task_lifecycle", "cases":cases}),
    );
    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}

// ---------- subagents browser capabilities differential ----------

#[test]
fn reference_subagents_browser_capabilities() {
    let fixture = load_fixture("subagents");
    let known = [
        "core", "pdf", "vision", "devtools", "config", "network", "storage", "testing",
    ];
    let mut cases = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let mut requested = Vec::new();
        for value in case["requested"].as_array().unwrap() {
            let name = value.as_str().unwrap().to_string();
            if !name.is_empty() && !requested.contains(&name) {
                requested.push(name);
            }
        }
        let rejected = requested
            .iter()
            .filter(|name| !known.contains(&name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let selected = std::iter::once("core".to_string())
            .chain(
                requested
                    .iter()
                    .filter(|name| known.contains(&name.as_str()) && name.as_str() != "core")
                    .cloned(),
            )
            .collect::<Vec<_>>();
        let valid_requested = requested
            .iter()
            .filter(|name| known.contains(&name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let allowed = ah_plugins_subagents::browser_tools_for_capabilities(&valid_requested)
            .expect("capabilities");
        cases.push(json!({"name":case["name"], "requested":requested, "selected":selected, "rejected":rejected, "allowed":allowed}));
    }
    settle("subagents", &json!({"seam":"subagents", "cases":cases}));
}

// ---------- evolving ----------

#[tokio::test]
async fn reference_evolving_evaluation() {
    let root = root_for("evolving");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_evolving::EvolvingPlugin::new(
        root.join("evolving"),
    )));
    let effects = mount(&ctx, plugins);
    let evolving = ctx
        .service::<dyn EvolvingRuntime>(&EVOLVING)
        .expect("evolving");
    let fixture = load_fixture("evolving");
    let input = &fixture["cases"][0]["input"];

    let mut events: Vec<SessionEvent> = Vec::new();
    for (i, ev) in input["events"].as_array().unwrap().iter().enumerate() {
        let kind = match ev["kind"].as_str().unwrap() {
            "user" => SessionEventKind::User,
            "assistant" => SessionEventKind::Assistant,
            "tool_result" => SessionEventKind::ToolResult,
            "agent_step" => SessionEventKind::AgentStep,
            other => panic!("unknown kind {other}"),
        };
        let payload = if kind == SessionEventKind::AgentStep {
            json!({"iteration": ev["iteration"], "tool_calls": ev["tool_calls"], "done": ev["done"]})
        } else if kind == SessionEventKind::ToolResult {
            json!({"tool_call_id": ev["tool_call_id"], "output": ev["output"]})
        } else if ev.get("tool_calls").is_some() {
            json!({"tool_calls": ev["tool_calls"]})
        } else {
            json!({"content": ev["content"]})
        };
        events.push(SessionEvent {
            seq: i as u64,
            timestamp_ms: (i as u64) * 1000,
            kind,
            payload,
        });
    }
    let traj = evolving
        .extract_trajectory("list workspace", &events)
        .expect("extract");
    let eval = evolving.evaluate(&traj).await.expect("evaluate");
    let round = |v: f64| (v * 10000.0).round() / 10000.0;
    let mut issues = eval.issues.clone();
    issues.sort();
    let mut strengths = eval.strengths.clone();
    strengths.sort();
    let outcome: Value = json!({
        "verdict": format!("{:?}", eval.verdict).to_lowercase(),
        "score": round(eval.score),
        "strengths": strengths,
        "issues": issues,
    });
    settle("evolving", &outcome);
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- stop_condition (P0-01 Python/Rust differential) ----------

fn stop_ctx(v: &Value) -> ah_contracts::harness_schema::StopEvaluationContext {
    ah_contracts::harness_schema::StopEvaluationContext {
        iteration: v.get("iteration").and_then(Value::as_u64).unwrap_or(0),
        token_usage: v.get("token_usage").and_then(Value::as_u64).unwrap_or(0),
        elapsed_seconds: v
            .get("elapsed_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        last_result: v.get("last_result").cloned(),
        extra: v
            .get("extra")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    }
}

/// 与 Python 侧(differential/run_python.py::run_stop_condition)相同的
/// 语言中立 fixture 与规范化输出形状。Python 输出见
/// differential/out/python/stop_condition.json,比较见 differential/compare.py。
#[test]
fn reference_stop_condition() {
    use ah_contracts::harness_schema::{
        CompletionPromiseEvaluator, MaxRoundsEvaluator, TimeoutEvaluator, TokenBudgetEvaluator,
    };

    let fixture = load_fixture("stop_condition");
    let mut cases_out: Vec<Value> = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let ev = &case["evaluator"];
        let name = case["name"].as_str().unwrap().to_string();
        match ev["type"].as_str().unwrap() {
            "max_rounds" => {
                let e = MaxRoundsEvaluator::new(ev["max_rounds"].as_u64().unwrap());
                let decisions: Vec<bool> = case["contexts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ctx| e.should_stop(&stop_ctx(ctx)))
                    .collect();
                cases_out.push(json!({ "name": name, "decisions": decisions }));
            }
            "token_budget" => {
                let e = TokenBudgetEvaluator::new(ev["max_tokens"].as_u64().unwrap());
                let decisions: Vec<bool> = case["contexts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ctx| e.should_stop(&stop_ctx(ctx)))
                    .collect();
                cases_out.push(json!({ "name": name, "decisions": decisions }));
            }
            "timeout" => {
                let e = TimeoutEvaluator::new(ev["timeout_seconds"].as_f64().unwrap());
                let decisions: Vec<bool> = case["contexts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|ctx| e.should_stop(&stop_ctx(ctx)))
                    .collect();
                cases_out.push(json!({ "name": name, "decisions": decisions }));
            }
            "completion_promise" => {
                let mut e = CompletionPromiseEvaluator::new(
                    ev["promise"].as_str().unwrap(),
                    ev["required_confirmations"].as_u64().unwrap(),
                );
                let mut results: Vec<Value> = Vec::new();
                for step in case["steps"].as_array().unwrap() {
                    match step["op"].as_str().unwrap() {
                        "notify_fulfilled" => e.notify_fulfilled(step["text"].as_str().unwrap()),
                        "notify_absent" => e.notify_absent(),
                        "should_stop" => results.push(Value::Bool(e.should_stop(
                            &ah_contracts::harness_schema::StopEvaluationContext::default(),
                        ))),
                        "get_state" => results.push(serde_json::to_value(e.get_state()).unwrap()),
                        "load_state" => e.load_state(&step["state"]),
                        "reset" => e.reset(),
                        other => panic!("unknown op {other}"),
                    }
                }
                cases_out.push(json!({ "name": name, "results": results }));
            }
            other => panic!("unknown evaluator type {other}"),
        }
    }
    settle(
        "stop_condition",
        &json!({ "seam": "stop_condition", "cases": cases_out }),
    );
}

// ---------- messager_inprocess (P0-01 Python/Rust differential) ----------

/// 与 Python 侧(differential/run_python.py::run_messager_inprocess)相同的
/// 语言中立 fixture 与规范化输出形状。Python 输出见
/// differential/out/python/messager_inprocess.json,比较见 differential/compare.py。
/// fixture 中 marked known_divergence 的 case 记录 Python/Rust 的真实语义差异
/// (进程全局总线 vs 每实例总线),由 compare.py 报告但不视为失败。
#[test]
fn reference_messager_inprocess() {
    use ah_contracts::messager::{Messager, MessagerTransportConfig, create_messager};
    use std::sync::{Arc, Mutex};

    let fixture = load_fixture("messager_inprocess");
    let mut cases_out: Vec<Value> = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let deliveries: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let leader_cfg = MessagerTransportConfig {
            node_id: Some(case["config"]["node_id"].as_str().unwrap().to_string()),
            ..Default::default()
        };
        let leader = create_messager(leader_cfg).expect("leader messager");
        let peer = case.get("peer_config").map(|pc| {
            create_messager(MessagerTransportConfig {
                node_id: Some(pc["node_id"].as_str().unwrap().to_string()),
                ..Default::default()
            })
            .expect("peer messager")
        });

        for step in case["steps"].as_array().unwrap() {
            let op = step["op"].as_str().unwrap();
            let who = step.get("who").and_then(Value::as_str).unwrap_or("leader");
            let m: &dyn Messager = if who == "leader" {
                leader.as_ref()
            } else {
                peer.as_ref().expect("peer").as_ref()
            };
            match op {
                "subscribe" => {
                    let d = deliveries.clone();
                    m.subscribe(step["topic"].as_str().unwrap(), handler(d));
                }
                "unsubscribe" => m.unsubscribe(step["topic"].as_str().unwrap()),
                "publish" => m.publish(step["topic"].as_str().unwrap(), step["message"].clone()),
                "send" => m.send(step["agent_id"].as_str().unwrap(), step["message"].clone()),
                "register_direct_message_handler" => {
                    let d = deliveries.clone();
                    m.register_direct_message_handler(handler(d));
                }
                "unregister_direct_message_handler" => m.unregister_direct_message_handler(),
                other => panic!("unknown op {other}"),
            }
        }

        let got: Vec<Value> = deliveries.lock().unwrap().clone();
        let mut entry = json!({ "name": case["name"], "deliveries": got });
        if case.get("known_divergence").is_some() {
            entry["known_divergence"] = Value::Bool(true);
        }
        cases_out.push(entry);
    }
    settle(
        "messager_inprocess",
        &json!({ "seam": "messager_inprocess", "cases": cases_out }),
    );
}

fn handler(
    deliveries: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
) -> ah_contracts::messager::MessagerHandler {
    Arc::new(move |msg: Value| {
        deliveries.lock().unwrap().push(msg);
    })
}

// ---------- application/controller P1 differential references ----------

#[tokio::test]
async fn reference_application_controller_contracts() {
    use ah_contracts::agent::{AgentRequest, ApplicationRuntime};
    use ah_contracts::controller::{Controller, Task, TaskFilter, TaskStatus};
    use ah_contracts::keys::APPLICATION;
    use ah_plugins_agent_control::AgentControlPlugin;
    use ah_plugins_agent_loop::AgentLoopPlugin;
    use ah_plugins_application::ApplicationPlugin;
    use ah_plugins_model_backup::ModelBackupPlugin;
    use ah_plugins_workflow::WorkflowPlugin;

    let application_fixture = load_fixture("application");
    let root = root_for("application");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.extend([
        Arc::new(AgentControlPlugin) as DynPlugin,
        Arc::new(ModelBackupPlugin::new(Vec::new())) as DynPlugin,
        Arc::new(WorkflowPlugin) as DynPlugin,
        Arc::new(AgentLoopPlugin::default()) as DynPlugin,
        Arc::new(ApplicationPlugin) as DynPlugin,
    ]);
    let effects = mount(&ctx, plugins);
    let application = ctx.service::<dyn ApplicationRuntime>(&APPLICATION).unwrap();
    let mut application_cases = Vec::new();
    for case in application_fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let request = AgentRequest {
            session_id: input["session_id"].as_str().unwrap_or_default().to_string(),
            input: input["input"].as_str().unwrap_or_default().to_string(),
            workflow: input.get("workflow").cloned(),
            timeout_ms: input.get("timeout_ms").and_then(Value::as_u64),
            restore_checkpoint: None,
            command: None,
            user_id: None,
            model: None,
            temperature: None,
        };
        let outcome = match application.invoke(request).await {
            Ok(result) => json!({
                "state": format!("{:?}", result.state).to_lowercase(),
                "failure": result.failure.map(|failure| format!("{:?}", failure).to_lowercase()),
                "session_id": result.session_id,
                "answer": result.answer,
                "answer_contains": result.answer.as_deref().is_some_and(|answer| {
                    input.get("answer_contains").and_then(Value::as_str).is_some_and(|needle| answer.contains(needle))
                }),
            }),
            Err(error) => json!({
                "ok": false,
                "error_class": if error.0.contains("session_id and input") { "invalid_request" } else { "runtime_error" },
            }),
        };
        application_cases.push(json!({ "name": case["name"], "outcome": outcome }));
    }
    settle(
        "application",
        &json!({ "seam": "application", "cases": application_cases }),
    );
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);

    let controller_fixture = load_fixture("controller");
    let controller = ah_plugins_controller::LocalController::new();
    let mut controller_cases = Vec::new();
    for case in controller_fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let outcome = match case["category"].as_str().unwrap() {
            "lifecycle" => {
                for task in input["tasks"].as_array().unwrap() {
                    controller
                        .create_task(Task::submitted(
                            task["session_id"].as_str().unwrap(),
                            task["id"].as_str().unwrap(),
                            task["task_type"].as_str().unwrap(),
                            task["description"].as_str().unwrap(),
                            task["priority"].as_i64().unwrap() as i32,
                        ))
                        .unwrap();
                }
                for link in input["parent_links"].as_array().unwrap() {
                    controller
                        .link_parent(
                            link["child"].as_str().unwrap(),
                            link["parent"].as_str().unwrap(),
                        )
                        .unwrap();
                }
                for transition in input["transitions"].as_array().unwrap() {
                    let status: TaskStatus =
                        serde_json::from_value(transition["to"].clone()).unwrap();
                    controller
                        .update_status(transition["id"].as_str().unwrap(), status)
                        .unwrap();
                }
                let tasks = controller
                    .filter_tasks(&TaskFilter {
                        session_id: Some("s1".into()),
                        ..Default::default()
                    })
                    .unwrap();
                json!({
                    "tasks": tasks.into_iter().map(|task| json!({
                        "id": task.task_id,
                        "status": serde_json::to_value(task.status).unwrap(),
                        "parent_task_id": task.parent_task_id,
                    })).collect::<Vec<_>>(),
                    "pending_ids": controller.pending_tasks("s1").into_iter().map(|task| task.task_id).collect::<Vec<_>>(),
                    "root_ids": controller.filter_tasks(&TaskFilter { is_root: true, ..Default::default() }).unwrap().into_iter().map(|task| task.task_id).collect::<Vec<_>>(),
                })
            }
            "illegal" => {
                let id = input["task_id"].as_str().unwrap();
                controller
                    .create_task(Task::submitted(
                        "illegal-session",
                        id,
                        "agent",
                        "illegal",
                        1,
                    ))
                    .unwrap();
                let status: TaskStatus = serde_json::from_value(input["to"].clone()).unwrap();
                json!({ "ok": controller.update_status(id, status).is_ok(), "error_class": "illegal_transition" })
            }
            "intent" => json!({
                "intents": input["queries"].as_array().unwrap().iter().map(|query| {
                    let intent = controller.recognize_intent(query["text"].as_str().unwrap());
                    json!({
                        "type": serde_json::to_value(intent.intent_type).unwrap(),
                        "has_task_text": intent.task_text.is_some(),
                        "confidence": intent.confidence,
                    })
                }).collect::<Vec<_>>()
            }),
            category => panic!("unknown controller category {category}"),
        };
        controller_cases.push(json!({ "name": case["name"], "outcome": outcome }));
    }
    settle(
        "controller",
        &json!({ "seam": "controller", "cases": controller_cases }),
    );
}

// ---------- llm_retry (Python/Rust differential) ----------

#[test]
fn reference_llm_retry() {
    let fixture = load_fixture("llm_retry");
    let cases = fixture["cases"].as_array().unwrap();
    let mut outcomes = cases
        .iter()
        .map(|case| {
            let text = case
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    case["unit"]
                        .as_str()
                        .unwrap()
                        .repeat(case["count"].as_u64().unwrap() as usize)
                });
            let detected = ah_plugins_rails::detect_repeated_suffix(&text)
                .map(|(unit, count)| json!({"unit": unit, "count": count}));
            json!({"name": case["name"], "detected": detected})
        })
        .collect::<Vec<_>>();
    for item in fixture["error_markers"].as_array().unwrap() {
        let message = item["message"].as_str().unwrap();
        outcomes.push(json!({
            "name": item["name"],
            "repeat": ah_plugins_rails::is_llm_repeat_error(message),
            "timeout": ah_plugins_rails::is_llm_stream_timeout_error(message)
        }));
    }
    let backoff_ms = fixture["backoff_indexes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|index| {
            ah_plugins_rails::llm_retry_backoff(
                &fixture["backoff_seconds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|seconds| (seconds.as_f64().unwrap() * 1000.0).round() as u64)
                    .collect::<Vec<_>>(),
                index.as_u64().unwrap() as usize,
            )
        })
        .collect::<Vec<_>>();
    outcomes.push(json!({"name": "backoff_schedule", "backoff_ms": backoff_ms}));
    settle(
        "llm_retry",
        &json!({"seam": "llm_retry", "cases": outcomes}),
    );
}

// ---------- tool_retry (Python/Rust differential) ----------

#[test]
fn reference_tool_retry() {
    let fixture = load_fixture("tool_retry");
    let cases = fixture["exceptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            json!({
                "name": item["name"],
                "retryable": ah_plugins_rails::is_tool_retryable_error(
                    item["type"].as_str().unwrap(),
                    item["message"].as_str().unwrap(),
                )
            })
        })
        .collect::<Vec<_>>();
    settle("tool_retry", &json!({"seam": "tool_retry", "cases": cases}));
}

// ---------- task_completion (Python/Rust differential) ----------

#[test]
fn reference_task_completion() {
    let fixture = load_fixture("task_completion");
    let cases = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            json!({
                "name": case["name"],
                "content": ah_plugins_rails::completion_signal_prompt(
                    case["language"].as_str().unwrap(),
                    case["promise"].as_str().unwrap(),
                ),
                "priority": 85
            })
        })
        .collect::<Vec<_>>();
    settle(
        "task_completion",
        &json!({"seam": "task_completion", "cases": cases}),
    );
}

// ---------- task_planning (Python/Rust differential) ----------

#[test]
fn reference_task_planning() {
    let fixture = load_fixture("task_planning");
    let cases = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let language = case["language"].as_str().unwrap();
            let models = case["models"]
                .as_array()
                .unwrap()
                .iter()
                .map(|model| {
                    (
                        model["id"].as_str().unwrap().to_string(),
                        model["description"].as_str().unwrap().to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let section = ah_plugins_prompt_builder::sections::base::build_todo_section_with_models(
                language, &models,
            );
            json!({
                "name": case["name"],
                "content": section.render(language),
                "priority": section.priority,
            })
        })
        .collect::<Vec<_>>();
    settle(
        "task_planning",
        &json!({"seam": "task_planning", "cases": cases}),
    );
}

// ---------- goal_manager (Python/Rust differential) ----------

fn goal_view(record: Option<&GoalRecord>) -> Value {
    let Some(record) = record else {
        return Value::Null;
    };
    json!({
        "objective": record.objective,
        "status": serde_json::to_value(record.status).unwrap(),
        "revision": record.revision,
        "attempt_count": record.attempt_count,
        "token_usage": {
            "input_tokens": record.token_usage.input_tokens,
            "output_tokens": record.token_usage.output_tokens,
            "cached_input_tokens": record.token_usage.cached_input_tokens,
            "total_tokens": record.token_usage.total_tokens,
        },
        "max_attempts": record.max_attempts,
        "token_budget": record.token_budget,
        "last_assessment": record.last_assessment.as_ref().map(|assessment| json!({
            "status": serde_json::to_value(assessment.status).unwrap(),
            "evidence": assessment.evidence,
            "remaining_work": assessment.remaining_work,
            "next_instruction": assessment.next_instruction,
        })),
        "last_stop_reason": record.last_stop_reason,
    })
}

#[test]
fn reference_goal_manager() {
    let fixture = load_fixture("goal_manager");
    let cases = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let manager = ah_plugins_rails::LocalGoalManager::new();
            let session_id = case["session_id"].as_str().unwrap();
            let mut goal_id = String::new();
            let mut revision = 0;
            let operations = case["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| {
                    let name = op["op"].as_str().unwrap();
                    match name {
                        "set" => match manager.set(
                            session_id,
                            op["objective"].as_str().unwrap(),
                            false,
                            op.get("max_attempts").and_then(Value::as_u64),
                            op.get("token_budget").and_then(Value::as_u64),
                        ) {
                            Ok(record) => {
                                goal_id = record.goal_id.clone();
                                revision = record.revision;
                                json!({"name": "set", "record": goal_view(Some(&record))})
                            }
                            Err(error) => json!({"name": "set_error", "code": error.code}),
                        },
                        "begin" => {
                            let record = manager.begin_attempt(session_id, &goal_id, revision);
                            if let Some(record) = &record {
                                revision = record.revision;
                            }
                            json!({"name": "begin", "record": goal_view(record.as_ref())})
                        }
                        "stale_begin" => json!({
                            "name": "stale_begin",
                            "record": goal_view(manager.begin_attempt(
                                session_id,
                                &goal_id,
                                revision.saturating_sub(1),
                            ).as_ref()),
                        }),
                        "usage" => {
                            manager.accumulate_usage(
                                session_id,
                                &goal_id,
                                revision,
                                op["input_tokens"].as_u64().unwrap_or(0),
                                op["output_tokens"].as_u64().unwrap_or(0),
                                op["cached_input_tokens"].as_u64().unwrap_or(0),
                            );
                            json!({"name": "usage", "record": goal_view(manager.get(session_id).as_ref())})
                        }
                        "pause" => json!({
                            "name": "pause",
                            "record": goal_view(manager.pause(session_id).as_ref()),
                        }),
                        "resume" => {
                            let record = manager.resume(session_id);
                            if let Some(record) = &record {
                                revision = record.revision;
                            }
                            json!({"name": "resume", "record": goal_view(record.as_ref())})
                        }
                        "continue" | "complete" | "block" => {
                            let status = match name {
                                "continue" => GoalAssessmentStatus::Continue,
                                "complete" => GoalAssessmentStatus::Complete,
                                "block" => GoalAssessmentStatus::Blocked,
                                _ => unreachable!(),
                            };
                            let record = manager.apply_assessment(
                                session_id,
                                &goal_id,
                                revision,
                                GoalAssessment {
                                    status,
                                    evidence: op["evidence"].as_str().unwrap_or_default().into(),
                                    remaining_work: op
                                        .get("remaining_work")
                                        .and_then(Value::as_str)
                                        .map(str::to_string),
                                    next_instruction: None,
                                },
                            );
                            json!({"name": name, "record": goal_view(record.as_ref())})
                        }
                        "clear" => json!({
                            "name": "clear",
                            "record": goal_view(manager.clear(session_id).as_ref()),
                        }),
                        other => panic!("unknown goal operation: {other}"),
                    }
                })
                .collect::<Vec<_>>();
            json!({"name": case["name"], "operations": operations})
        })
        .collect::<Vec<_>>();
    settle(
        "goal_manager",
        &json!({"seam": "goal_manager", "cases": cases}),
    );
}

// ---------- prompt_attachment (Python/Rust differential) ----------

fn prompt_attachment_view(
    item: Option<&ah_contracts::prompt_attachment::PromptAttachment>,
) -> Value {
    let Some(item) = item else {
        return Value::Null;
    };
    json!({
        "id": item.id,
        "section": item.section,
        "kind": serde_json::to_value(item.kind).unwrap(),
        "content": item.content,
        "priority": item.priority,
        "source": item.source,
        "session_id": item.session_id,
        "expires_at": item.expires_at,
        "metadata": item.metadata,
        "content_kind": item.content_kind,
        "content_sha256": item.content_sha256,
    })
}

#[test]
fn reference_prompt_attachment() {
    use ah_contracts::prompt_attachment::{
        AttachmentFilter, PromptAttachmentKind, PromptAttachmentStore, PromptAttachmentUpdate,
    };

    let fixture = load_fixture("prompt_attachment");
    let cases = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let store = ah_plugins_prompt_attachment::InMemoryPromptAttachmentStore::new();
            let session_id = case["session_id"].as_str().unwrap();
            let mut ids = std::collections::HashMap::new();
            let operations = case["operations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|op| {
                    let name = op["op"].as_str().unwrap();
                    match name {
                        "add" => {
                            let kind = match op["kind"].as_str().unwrap_or("generic") {
                                "text" => PromptAttachmentKind::Text,
                                "runtime" => PromptAttachmentKind::Runtime,
                                other => panic!("unknown prompt attachment kind: {other}"),
                            };
                            let metadata = op
                                .get("metadata")
                                .and_then(Value::as_object);
                            let item = store
                                .add_section(
                                    session_id,
                                    op["section"].as_str().unwrap(),
                                    op["content"].as_str().unwrap(),
                                    kind,
                                    op["source"].as_str().unwrap(),
                                    op["priority"].as_i64().unwrap() as i32,
                                    metadata,
                                    "text/plain",
                                    op.get("expires_at").and_then(Value::as_str),
                                )
                                .expect("add attachment");
                            ids.insert(op["name"].as_str().unwrap(), item.id.clone());
                            json!({"name": "add", "item": prompt_attachment_view(Some(&item))})
                        }
                        "update" => {
                            let id = ids.get(op["name"].as_str().unwrap()).unwrap();
                            let item = store
                                .update_by_id(
                                    id,
                                    &PromptAttachmentUpdate {
                                        kind: None,
                                        content: Some(op["content"].as_str().unwrap().to_string()),
                                        priority: None,
                                        source: None,
                                        expires_at: None,
                                        metadata: Some(
                                            op["metadata"].as_object().unwrap().clone(),
                                        ),
                                        content_kind: None,
                                    },
                                )
                                .expect("update attachment");
                            json!({"name": "update", "item": prompt_attachment_view(Some(&item))})
                        }
                        "list" => {
                            let items = store.list_by_filter(&AttachmentFilter {
                                session_id: Some(session_id.to_string()),
                                section: op.get("section").and_then(Value::as_str).map(str::to_string),
                                ..Default::default()
                            });
                            json!({
                                "name": "list",
                                "items": items.iter().map(|item| prompt_attachment_view(Some(item))).collect::<Vec<_>>(),
                            })
                        }
                        "collect" => json!({
                            "name": "collect",
                            "items": store
                                .collect_for_session(session_id)
                                .iter()
                                .map(|item| prompt_attachment_view(Some(item)))
                                .collect::<Vec<_>>(),
                        }),
                        "clear_section" => json!({
                            "name": "clear_section",
                            "count": store.clear_section(session_id, op["section"].as_str().unwrap()),
                        }),
                        "clear_session" => json!({
                            "name": "clear_session",
                            "count": store.clear_session(session_id),
                        }),
                        other => panic!("unknown prompt attachment operation: {other}"),
                    }
                })
                .collect::<Vec<_>>();
            json!({"name": case["name"], "operations": operations})
        })
        .collect::<Vec<_>>();
    settle(
        "prompt_attachment",
        &json!({"seam": "prompt_attachment", "cases": cases}),
    );
}

#[tokio::test]
async fn reference_runtime_model_switching() {
    struct NamedProvider(&'static str);

    impl ah_contracts::seam::Seam for NamedProvider {}

    #[async_trait]
    impl ModelProvider for NamedProvider {
        fn name(&self) -> &'static str {
            self.0
        }

        async fn chat(
            &self,
            _request: ModelRequest,
        ) -> Result<ModelResponse, ah_contracts::llm::ModelError> {
            Ok(ModelResponse {
                content: self.0.into(),
                ..Default::default()
            })
        }
    }

    let fixture = load_fixture("runtime_model_switching");
    let mut cases = Vec::new();
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let root = std::env::temp_dir().join(format!(
            "ah-differential-model-switch-{}-{index}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Context::new();
        let effects = ctx
            .mount_all(vec![
                Arc::new(ah_plugins_tools::ToolsPlugin) as ah_hub::plugin::DynPlugin,
                Arc::new(ah_plugins_session_log::SessionLogPlugin::new(
                    root.join("default.jsonl"),
                    &root,
                )),
            ])
            .expect("mount model switching differential context");
        let sessions = ctx
            .service::<dyn ah_contracts::session::SessionLog>(&ah_contracts::keys::SESSIONS)
            .expect("session log");
        let tools = ctx
            .service::<dyn ah_contracts::tools::ToolRegistry>(&ah_contracts::keys::TOOLS)
            .expect("tool registry");
        let providers = case["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| {
                let name = match model.as_str().unwrap() {
                    "default" => "default",
                    "alternate" => "alternate",
                    other => panic!("unexpected fixture model: {other}"),
                };
                Arc::new(NamedProvider(name)) as Arc<dyn ModelProvider>
            })
            .collect();
        let catalog = ah_plugins_model_backup::StaticModelProviderCatalog::new(providers)
            .expect("model catalog");
        let default_model = catalog
            .resolve(case["default_model"].as_str().unwrap())
            .expect("default model");
        let request = ah_contracts::agent::AgentRunConfig {
            model: case["selected_model_id"].as_str().map(str::to_string),
            ..Default::default()
        };
        let result =
            ah_plugins_agent_loop::AgentLoop::new(default_model, tools, sessions.clone(), ctx, 1)
                .with_model_catalog(Arc::new(catalog))
                .with_request_config(&request)
                .run_in_session(sessions, "switch model")
                .await;
        assert_eq!(
            result.state,
            ah_contracts::agent::AgentRunState::Completed,
            "{result:?}"
        );
        let selected_model = result.answer.expect("model response");
        cases.push(json!({
            "name": case["name"],
            "selected_model": selected_model,
            "config_model": selected_model,
        }));
        drop(effects);
        let _ = std::fs::remove_dir_all(root);
    }
    settle(
        "runtime_model_switching",
        &json!({"seam": "runtime_model_switching", "cases": cases}),
    );
}

#[test]
fn reference_team_inbox_format() {
    use ah_contracts::external_format::{ExternalFormat, MessageView, TaskLineView};
    use ah_plugins_external_format::ExternalFormatEngine;
    use ah_plugins_inbound_render::InboundRenderer;
    use ah_plugins_timefmt::TimefmtService;

    let fixture = load_fixture("team_inbox_format");
    let engine = ExternalFormatEngine;
    let render = InboundRenderer;
    let timefmt = TimefmtService::with_offset(0);
    let cases = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let output = if case["kind"] == "message" {
                let message = MessageView {
                    broadcast: case["broadcast"].as_bool().unwrap(),
                    timestamp: case["timestamp"].as_i64().unwrap(),
                    from_member_name: case["from_member_name"].as_str().unwrap().to_string(),
                    message_id: case["message_id"].as_str().unwrap().to_string(),
                    content: case["content"].as_str().unwrap().to_string(),
                };
                engine.render_message(
                    &message,
                    case["is_human_agent"].as_bool().unwrap(),
                    case["now_ms"].as_i64().unwrap(),
                    case["body"].as_str(),
                    case["reply_hint"].as_str(),
                    case["hitt_silence_note"].as_str(),
                    &render,
                    &timefmt,
                )
            } else {
                let tasks = case["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|task| TaskLineView {
                        task_id: task["task_id"].as_str().unwrap().to_string(),
                        title: task["title"].as_str().unwrap().to_string(),
                        content: task["content"].as_str().unwrap().to_string(),
                        status: task["status"].as_str().unwrap().to_string(),
                        assignee: task["assignee"].as_str().map(str::to_string),
                        updated_at: task["updated_at"].as_i64(),
                    })
                    .collect::<Vec<_>>();
                engine.render_task_board(
                    &tasks,
                    case["is_leader"].as_bool().unwrap(),
                    case["now_ms"].as_i64().unwrap(),
                    case["leader_header"].as_str().unwrap(),
                    case["teammate_header"].as_str().unwrap(),
                    case["unassigned_marker"].as_str().unwrap(),
                    &render,
                    &timefmt,
                )
            };
            json!({"name": case["name"], "output": output})
        })
        .collect::<Vec<_>>();
    settle(
        "team_inbox_format",
        &json!({"seam": "team_inbox_format", "cases": cases}),
    );
}

#[tokio::test]
async fn reference_team_inbox_fetch() {
    use ah_contracts::external_client::ExternalTeamClientFactory;
    use ah_contracts::external_client::{ExternalInboxSource, InboxMessage, InboxObserver};
    use ah_contracts::external_format::{MessageView, TaskLineView};
    use ah_contracts::team_join_descriptor::{
        DispatchMode, JoinDbConfig, JoinTransportConfig, Scope, TeamJoinDescriptor, TeammateMode,
    };
    use ah_contracts::team_message::{MemberView, TaskView};
    use ah_plugins_external::ExternalClientPlugin;
    use ah_plugins_external_format::ExternalFormatPlugin;
    use ah_plugins_inbound_render::InboundRenderer;
    use ah_plugins_team_i18n::TeamI18nPlugin;
    use ah_plugins_team_message::TeamMessagePlugin;
    use ah_plugins_team_prompts::TeamPromptsPlugin;
    use ah_plugins_timefmt::TimefmtService;

    struct Source {
        direct: Vec<InboxMessage>,
        broadcast: Vec<InboxMessage>,
        tasks: Vec<TaskLineView>,
        marked: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl ah_contracts::seam::Seam for Source {}

    #[async_trait]
    impl ExternalInboxSource for Source {
        async fn unread_direct_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            self.direct.clone()
        }

        async fn unread_broadcast_messages(&self, _member_name: &str) -> Vec<InboxMessage> {
            self.broadcast.clone()
        }

        async fn mark_message_read(&self, message_id: &str, _member_name: &str) {
            self.marked.lock().unwrap().push(message_id.to_string());
        }

        async fn list_tasks(&self) -> Vec<TaskLineView> {
            self.tasks.clone()
        }

        async fn get_task(&self, _task_id: &str) -> Option<TaskView> {
            None
        }

        async fn get_member(&self, _member_name: &str) -> Option<MemberView> {
            None
        }
    }

    fn message(id: &str) -> InboxMessage {
        InboxMessage {
            view: MessageView {
                broadcast: false,
                timestamp: 1_700_000_000_000,
                from_member_name: "alice".into(),
                message_id: id.into(),
                content: id.into(),
            },
            meta: None,
        }
    }

    let ctx = Context::new();
    let render: Arc<dyn ah_contracts::inbound_render::InboundRender> = Arc::new(InboundRenderer);
    let timefmt: Arc<dyn ah_contracts::timefmt::Timefmt> = Arc::new(TimefmtService::with_offset(0));
    let e1 = ctx.register(ah_contracts::keys::INBOUND_RENDER, render);
    struct Observer {
        counts: Arc<std::sync::Mutex<usize>>,
    }

    #[async_trait]
    impl InboxObserver for Observer {
        async fn on_inbox(&self, _view: ah_contracts::external_client::InboxView) {
            *self.counts.lock().unwrap() += 1;
        }
    }
    let e2 = ctx.register(ah_contracts::keys::TIMEFMT, timefmt);
    let messager =
        ah_contracts::messager::create_messager(ah_contracts::messager::MessagerTransportConfig {
            node_id: Some("differential-fetch".into()),
            ..Default::default()
        })
        .expect("messager");
    let e3 = ctx.register(ah_contracts::keys::MESSAGER, messager);
    let plugins: Vec<ah_hub::plugin::DynPlugin> = vec![
        Arc::new(ah_plugins_team_context::TeamContextPlugin),
        Arc::new(TeamPromptsPlugin),
        Arc::new(TeamMessagePlugin),
        Arc::new(TeamI18nPlugin),
        Arc::new(ExternalFormatPlugin),
        Arc::new(ExternalClientPlugin),
    ];
    let mut effects = ctx.mount_all(plugins).expect("mount external client");
    effects.extend([e1, e2, e3]);
    let factory = ctx
        .service::<dyn ExternalTeamClientFactory>(&ah_contracts::keys::EXTERNAL_CLIENT)
        .expect("external client");
    let descriptor = TeamJoinDescriptor {
        session_id: "s1".into(),
        team_name: "t1".into(),
        member_name: "dev-1".into(),
        role: "teammate".into(),
        scope: Scope::Member,
        language: "cn".into(),
        dispatch_mode: DispatchMode::Autonomous,
        teammate_mode: TeammateMode::BuildMode,
        db_config: JoinDbConfig::default(),
        transport_config: JoinTransportConfig::default(),
        workspace_path: None,
    };
    let fixture = load_fixture("team_inbox_fetch");
    let mut cases = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        if case.get("kind").and_then(Value::as_str) == Some("watch") {
            continue;
        }
        let marked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let source = Arc::new(Source {
            direct: case["direct_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| message(id.as_str().unwrap()))
                .collect(),
            broadcast: case["broadcast_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| message(id.as_str().unwrap()))
                .collect(),
            tasks: case["task_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| TaskLineView {
                    task_id: id.as_str().unwrap().into(),
                    title: id.as_str().unwrap().into(),
                    content: id.as_str().unwrap().into(),
                    status: "pending".into(),
                    assignee: None,
                    updated_at: None,
                })
                .collect(),
            marked: marked.clone(),
        });
        let client = factory.build(&descriptor);
        client.connect(source).expect("connect");
        let view = client
            .fetch_inbox(case["mark_read"].as_bool().unwrap())
            .await
            .expect("fetch");
        cases.push(json!({
            "name": case["name"],
            "message_ids": view.messages.iter().map(|message| message.view.message_id.clone()).collect::<Vec<_>>(),
            "task_ids": view.tasks.iter().map(|task| task.task_id.clone()).collect::<Vec<_>>(),
            "marked_ids": marked.lock().unwrap().clone(),
        }));
        client.close().expect("close");
    }
    let watch_marked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let watch_source = Arc::new(Source {
        direct: vec![message("watch-1")],
        broadcast: vec![],
        tasks: vec![],
        marked: watch_marked.clone(),
    });
    let watch_client = factory.build(&descriptor);
    watch_client.connect(watch_source).expect("watch connect");
    let callback_count = Arc::new(std::sync::Mutex::new(0usize));
    let watch_task = tokio::spawn({
        let watch_client = watch_client.clone();
        let callback_count = callback_count.clone();
        async move {
            watch_client
                .watch(&Observer {
                    counts: callback_count,
                })
                .await
        }
    });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let messager = ctx
        .service::<dyn ah_contracts::messager::Messager>(&ah_contracts::keys::MESSAGER)
        .expect("messager");
    let topic = ah_contracts::team_schema::TeamTopic::Message.build("s1", "t1");
    messager.publish(&topic, json!({"event": "message"}));
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if *callback_count.lock().unwrap() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("watch callback");
    watch_task.abort();
    let _ = watch_task.await;
    let count_after_cancel = *callback_count.lock().unwrap();
    messager.publish(&topic, json!({"event": "after-cancel"}));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(*callback_count.lock().unwrap(), count_after_cancel);
    cases.push(json!({
        "name": "watch_cancellation",
        "callback_count": count_after_cancel,
        "marked_ids": watch_marked.lock().unwrap().clone(),
        "unsubscribed": true,
    }));
    watch_client.close().expect("watch close");
    settle(
        "team_inbox_fetch",
        &json!({"seam": "team_inbox_fetch", "cases": cases}),
    );
}
