//! 契约 golden fixtures(G-02):语言中立 fixtures 驱动真实 seam 行为验证。
//!
//! fixtures/ 目录下的 JSON 定义输入与期望输出;本文件把每个 seam 的真实插件
//! 实现喂给 fixtures,断言行为与 golden 一致。fixtures 不依赖 Python 代码。

use std::path::PathBuf;
use std::sync::Arc;

use ah_contracts::evolving::{EvolvingRuntime, Verdict};
use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{
    EVOLVING, FS, MEMORY, RETRIEVAL, RSI, SECURITY, SESSION_MANAGER, TEAMS, TOOLS,
};
use ah_contracts::memory::MemoryProvider;
use ah_contracts::retrieval::RetrievalProvider;
use ah_contracts::rsi::RsiRuntime;
use ah_contracts::security::SecurityProvider;
use ah_contracts::session::{SessionEvent, SessionEventKind, SessionLog, SessionManager};
use ah_contracts::teams::{TeamRuntime, TeamSpec, TeamTask, TeamTaskStatus};
use ah_contracts::tools::ToolRegistry;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use serde_json::{Value, json};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fixtures")
}

fn load_fixture(name: &str) -> Value {
    let text = std::fs::read_to_string(fixtures_dir().join(format!("{name}.json")))
        .unwrap_or_else(|e| panic!("load fixture {name}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse fixture {name}: {e}"))
}

fn root_for(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ah-golden-{tag}-{}", std::process::id()))
}

fn mount(ctx: &Context, plugins: Vec<DynPlugin>) -> Vec<ah_contracts::Effect> {
    ctx.mount_all(plugins).expect("mount")
}

/// 常用插件组合:mock + tools + sysop + session-log。
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

// ---------- fs ----------

#[test]
fn fs_golden() {
    let root = root_for("fs");
    let ctx = Context::new();
    let effects = mount(
        &ctx,
        vec![
            Arc::new(ah_plugins_tools::ToolsPlugin),
            Arc::new(ah_plugins_sysop::SysopPlugin::new(&root)),
        ],
    );
    let fs = ctx.service::<dyn FsProvider>(&FS).expect("fs");
    let fixture = load_fixture("fs");

    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let expect = &case["expect"];
        match case["name"].as_str().unwrap() {
            "write_read_roundtrip" => {
                fs.write(
                    input["rel"].as_str().unwrap(),
                    input["content"].as_str().unwrap().as_bytes(),
                )
                .expect("write");
                let content = fs.read(input["rel"].as_str().unwrap()).expect("read");
                assert_eq!(
                    String::from_utf8(content).unwrap(),
                    expect["content"].as_str().unwrap(),
                    "fs roundtrip content"
                );
                assert!(fs.list("a").unwrap().contains(&"b.txt".to_string()));
                assert!(fs.exists(input["rel"].as_str().unwrap()));
            }
            "remove_then_gone" => {
                fs.remove(input["rel"].as_str().unwrap()).expect("remove");
                assert!(!fs.exists(input["rel"].as_str().unwrap()));
            }
            "escape_denied" => {
                assert!(
                    fs.read(input["rel"].as_str().unwrap()).is_err(),
                    "escape must be denied"
                );
            }
            other => panic!("unknown fs case: {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- session ----------

#[test]
fn session_golden() {
    let root = root_for("session");
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins(&root));
    let manager = ctx
        .service::<dyn SessionManager>(&SESSION_MANAGER)
        .expect("manager");
    let fixture = load_fixture("session");

    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        match case["name"].as_str().unwrap() {
            "append_and_derive_messages" => {
                let log = manager.create("g1").expect("create");
                for ev in input["events"].as_array().unwrap() {
                    let kind = match ev["kind"].as_str().unwrap() {
                        "user" => SessionEventKind::User,
                        "assistant" => SessionEventKind::Assistant,
                        "tool_result" => SessionEventKind::ToolResult,
                        "agent_step" => SessionEventKind::AgentStep,
                        other => panic!("unknown kind {other}"),
                    };
                    let payload = json!({ "content": ev["content"].as_str().unwrap() });
                    log.append(kind, payload).expect("append");
                }
                let messages = log.derive_messages();
                let roles: Vec<&str> = messages
                    .iter()
                    .map(|m| match m.role {
                        ah_contracts::llm::ChatRole::User => "user",
                        ah_contracts::llm::ChatRole::Assistant => "assistant",
                        _ => "other",
                    })
                    .collect();
                assert_eq!(roles, expect_roles(case), "derived roles");
            }
            "jsonl_roundtrip_resume" => {
                let log = manager.create("g2").expect("create");
                for ev in input["events"].as_array().unwrap() {
                    log.append(
                        SessionEventKind::User,
                        json!({ "content": ev["content"].as_str().unwrap() }),
                    )
                    .expect("append");
                }
                // 真实磁盘续跑:同一文件重新打开,事件从 JSONL 恢复。
                let path = root.join("sessions").join("g2.jsonl");
                let resumed = ah_plugins_session_log::JsonlSessionLog::open(&path, ctx.clone())
                    .expect("reopen");
                let count = resumed.events().len();
                assert_eq!(
                    count,
                    case["expect"]["resumed_event_count"].as_u64().unwrap() as usize
                );
            }
            other => panic!("unknown session case: {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

fn expect_roles(case: &Value) -> Vec<&'static str> {
    case["expect"]["derived_roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| match r.as_str().unwrap() {
            "user" => "user",
            "assistant" => "assistant",
            other => panic!("unknown expected role {other}"),
        })
        .collect()
}

// ---------- tools ----------

#[tokio::test]
async fn tools_golden() {
    let root = root_for("tools");
    let ctx = Context::new();
    let effects = mount(&ctx, base_plugins(&root));
    let fs = ctx.service::<dyn FsProvider>(&FS).expect("fs");
    let registry = ctx.service::<dyn ToolRegistry>(&TOOLS).expect("tools");
    let fixture = load_fixture("tools");

    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let expect = &case["expect"];
        match case["name"].as_str().unwrap() {
            "invoke_real_read_file" => {
                fs.write(
                    input["file"].as_str().unwrap(),
                    input["content"].as_str().unwrap().as_bytes(),
                )
                .expect("write probe");
                let output = registry
                    .invoke(
                        input["tool"].as_str().unwrap(),
                        json!({ "path": input["file"].as_str().unwrap() }),
                    )
                    .await
                    .expect("invoke");
                assert!(
                    output
                        .to_string()
                        .contains(expect["output_contains"].as_str().unwrap()),
                    "tool output must contain expected content"
                );
            }
            "unknown_tool_rejected" => {
                assert!(
                    registry
                        .invoke(input["tool"].as_str().unwrap(), json!({}))
                        .await
                        .is_err(),
                    "unknown tool must error"
                );
            }
            other => panic!("unknown tools case: {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- memory ----------

#[test]
fn memory_golden() {
    let root = root_for("memory");
    let dir = root.join("mem");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_memory::MemoryPlugin::new(&dir)));
    let effects = mount(&ctx, plugins);
    let memory = ctx.service::<dyn MemoryProvider>(&MEMORY).expect("memory");
    let fixture = load_fixture("memory");
    let input = &fixture["cases"][0]["input"];

    memory
        .store(
            input["key"].as_str().unwrap(),
            input["content"].as_str().unwrap(),
            vec!["project".to_string()],
        )
        .expect("store");
    let record = memory
        .retrieve(input["key"].as_str().unwrap())
        .expect("retrieve");
    assert_eq!(
        record.content,
        fixture["cases"][0]["expect"]["retrieved"].as_str().unwrap()
    );
    assert!(
        memory
            .search("project")
            .iter()
            .any(|r| r.key == input["key"].as_str().unwrap()),
        "search must find by tag"
    );

    // 序列化/恢复:同一目录重开 provider,记忆从磁盘恢复。
    let reopened = ah_plugins_memory::JsonFileMemoryProvider::open(&dir).expect("reopen");
    let restored = reopened
        .retrieve(input["key"].as_str().unwrap())
        .expect("restored");
    assert_eq!(
        restored.content,
        fixture["cases"][1]["expect"]["restored"].as_str().unwrap()
    );

    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- retrieval ----------

#[test]
fn retrieval_golden() {
    let root = root_for("retrieval");
    let dir = root.join("kb");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_retrieval::RetrievalPlugin::new(&dir)));
    let effects = mount(&ctx, plugins);
    let kb = ctx
        .service::<dyn RetrievalProvider>(&RETRIEVAL)
        .expect("retrieval");
    let fixture = load_fixture("retrieval");
    let case = &fixture["cases"][0];
    let input = &case["input"];

    kb.ingest(
        input["doc_id"].as_str().unwrap(),
        input["text"].as_str().unwrap(),
        json!({}),
    )
    .expect("ingest");
    match case["name"].as_str().unwrap() {
        "ingest_and_search_hit" => {
            let hits = kb.retrieve(
                input["query"].as_str().unwrap(),
                case["expect"]["k"].as_u64().unwrap() as usize,
            );
            assert!(!hits.is_empty(), "must have hits");
            assert_eq!(hits[0].doc_id, case["expect"]["hit_doc"].as_str().unwrap());
            assert!(
                hits[0]
                    .chunk
                    .contains(case["expect"]["hit_contains"].as_str().unwrap())
            );
        }
        "vector_search_hit" => {
            let hits = kb.retrieve_vector(
                input["query"].as_str().unwrap(),
                case["expect"]["k"].as_u64().unwrap() as usize,
            );
            assert!(!hits.is_empty(), "vector path must have hits");
            assert_eq!(hits[0].doc_id, case["expect"]["hit_doc"].as_str().unwrap());
            assert!(
                (0.0..=1.0).contains(&hits[0].score),
                "vector score normalized to [0,1]"
            );
        }
        other => panic!("unknown retrieval case: {other}"),
    }

    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- security ----------

#[test]
fn security_golden() {
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

    for case in fixture["cases"].as_array().unwrap() {
        let verdict = security.verdict(case["input"]["content"].as_str().unwrap());
        assert_eq!(
            verdict.allow,
            case["expect"]["allow"].as_bool().unwrap(),
            "case {} allow",
            case["name"].as_str().unwrap()
        );
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- evolving ----------

fn ev(seq: u64, kind: SessionEventKind, payload: Value) -> SessionEvent {
    SessionEvent {
        seq,
        timestamp_ms: seq * 1000,
        kind,
        payload,
    }
}

#[tokio::test]
async fn evolving_golden() {
    let root = root_for("evolving");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_evolving::EvolvingPlugin));
    let effects = mount(&ctx, plugins);
    let evolving = ctx
        .service::<dyn EvolvingRuntime>(&EVOLVING)
        .expect("evolving");
    let fixture = load_fixture("evolving");

    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let expect = &case["expect"];
        let mut events: Vec<SessionEvent> = Vec::new();
        for (i, ev_def) in input["events"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .enumerate()
        {
            let kind = match ev_def["kind"].as_str().unwrap() {
                "user" => SessionEventKind::User,
                "assistant" => SessionEventKind::Assistant,
                "tool_result" => SessionEventKind::ToolResult,
                "agent_step" => SessionEventKind::AgentStep,
                other => panic!("unknown evolving event kind {other}"),
            };
            let payload = if kind == SessionEventKind::AgentStep {
                json!({
                    "iteration": ev_def["iteration"].as_u64().unwrap(),
                    "tool_calls": ev_def["tool_calls"].as_u64().unwrap(),
                    "done": ev_def["done"].as_bool().unwrap(),
                })
            } else if kind == SessionEventKind::ToolResult {
                json!({
                    "tool_call_id": ev_def["tool_call_id"].as_str().unwrap(),
                    "output": ev_def["output"].as_str().unwrap(),
                })
            } else if ev_def.get("tool_calls").is_some() {
                json!({ "tool_calls": ev_def["tool_calls"].clone() })
            } else {
                json!({ "content": ev_def["content"].as_str().unwrap() })
            };
            events.push(ev(i as u64, kind, payload));
        }
        let traj = evolving
            .extract_trajectory(input["task"].as_str().unwrap(), &events)
            .expect("extract");
        let eval = evolving.evaluate(&traj).await.expect("evaluate");
        match expect["verdict"].as_str().unwrap() {
            "pass" => {
                assert_eq!(
                    eval.verdict,
                    Verdict::Pass,
                    "case {}: {:?}",
                    case["name"],
                    eval
                );
                assert!(eval.score >= expect["score_min"].as_f64().unwrap());
            }
            "fail" => {
                assert_eq!(
                    eval.verdict,
                    Verdict::Fail,
                    "case {}: {:?}",
                    case["name"],
                    eval
                );
                assert!(eval.score <= expect["score_max"].as_f64().unwrap());
            }
            other => panic!("unknown expected verdict {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- teams ----------

fn team_task(id: &str, title: &str, deps: Vec<String>) -> TeamTask {
    TeamTask {
        id: id.to_string(),
        title: title.to_string(),
        status: TeamTaskStatus::Pending,
        dependencies: deps,
        assignee: None,
        review_votes: Vec::new(),
        result: None,
    }
}

#[tokio::test]
async fn teams_golden() {
    let root = root_for("teams");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_subagent::SubagentPlugin));
    plugins.push(Arc::new(ah_plugins_teams::TeamsPlugin));
    let effects = mount(&ctx, plugins);
    let teams = ctx.service::<dyn TeamRuntime>(&TEAMS).expect("teams");
    let fixture = load_fixture("teams");

    for (case_idx, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let input = &case["input"];
        // 每个用例独立团队,避免跨用例状态污染。
        let team_id = format!("{}-{case_idx}", input["team"]["id"].as_str().unwrap());
        let spec = TeamSpec {
            id: team_id.clone(),
            name: input["team"]["name"].as_str().unwrap().to_string(),
        };
        let members: Vec<_> = input["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| ah_contracts::teams::TeamMemberSpec {
                id: m["id"].as_str().unwrap().to_string(),
                name: m["name"].as_str().unwrap().to_string(),
                role: m["role"].as_str().unwrap().to_string(),
            })
            .collect();
        teams.create_team(spec, members).expect("create team");
        let t = &input["task"];
        teams
            .add_task(
                &team_id,
                team_task(
                    t["id"].as_str().unwrap(),
                    t["title"].as_str().unwrap(),
                    vec![],
                ),
            )
            .expect("add task");
        match case["name"].as_str().unwrap() {
            "lifecycle_to_done" => {
                teams.claim_task(&team_id, "m1").expect("claim");
                teams
                    .complete_task(&team_id, t["id"].as_str().unwrap(), json!({ "ok": true }))
                    .expect("complete");
                let tasks = teams.tasks(&team_id).expect("tasks");
                assert_eq!(tasks[0].status, TeamTaskStatus::Done, "lifecycle to done");
            }
            "non_member_claim_rejected" => {
                assert!(
                    teams
                        .claim_task(&team_id, input["claimant"].as_str().unwrap())
                        .is_err(),
                    "non-member claim must be rejected"
                );
            }
            other => panic!("unknown teams case: {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- rsi ----------

#[tokio::test]
async fn rsi_golden() {
    let root = root_for("rsi");
    let ctx = Context::new();
    let mut plugins = base_plugins(&root);
    plugins.push(Arc::new(ah_plugins_subagent::SubagentPlugin));
    plugins.push(Arc::new(ah_plugins_evolving::EvolvingPlugin));
    plugins.push(Arc::new(ah_plugins_rsi::RsiPlugin::new(root.join("rsi"))));
    let effects = mount(&ctx, plugins);
    let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi");
    let fixture = load_fixture("rsi");

    for case in fixture["cases"].as_array().unwrap() {
        let input = &case["input"];
        let seeds: Vec<String> = input["seed_tasks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        match case["name"].as_str().unwrap() {
            "dataset_expansion_count" => {
                let cases = rsi
                    .generate_dataset(seeds, input["count"].as_u64().unwrap() as usize)
                    .expect("gen");
                assert_eq!(
                    cases.len(),
                    case["expect"]["count"].as_u64().unwrap() as usize
                );
                let mut ids: Vec<String> = cases.iter().map(|c| c.id.clone()).collect();
                ids.sort();
                ids.dedup();
                assert_eq!(
                    ids.len(),
                    case["expect"]["unique_ids"].as_u64().unwrap() as usize
                );
            }
            "empty_seeds_rejected" => {
                assert!(
                    rsi.generate_dataset(seeds, 1).is_err(),
                    "empty seeds must error"
                );
            }
            other => panic!("unknown rsi case: {other}"),
        }
    }
    drop(effects);
    let _ = std::fs::remove_dir_all(&root);
}
