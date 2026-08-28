//! 差分契约 references(语言中立行为快照)。
//!
//! 对同一组 fixture 输入,把真实实现的**完整可观测输出**固化为 references/{seam}.json
//! (Rust 基线;按 testing.md §3,外部 Python 参考运行将验证/覆盖这些快照)。
//! 默认运行:断言 Rust 行为与 references 一致(回归保护);
//! 设 AH_REFGEN=1 运行:重新生成 references(快照更新)。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::evolving::EvolvingRuntime;
use ah_contracts::keys::{EVOLVING, RETRIEVAL, SECURITY, SESSION_MANAGER, TEAMS};
use ah_contracts::retrieval::RetrievalProvider;
use ah_contracts::security::SecurityProvider;
use ah_contracts::session::{SessionEvent, SessionEventKind, SessionManager};
use ah_contracts::teams::TeamRuntime;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
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
                status: ah_contracts::teams::TeamTaskStatus::Pending,
                dependencies: vec![],
                assignee: None,
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
