//! agent-harness 交互 CLI:输入任务跑 agent,会话可新建/切换/分叉;
//! 子命令覆盖全部真实 seam(teams/rsi/workspace/web/queue/code)。

use std::io::{BufRead, Write};

use ah_contracts::keys::{CODE, QUEUE, RSI, SESSIONS, TEAMS, WEB, WORKSPACE};
use ah_contracts::session::SessionLog;
use ah_contracts::workspace::{GoalStatus, WorkspaceService};
use ah_plugins_agent_loop::AgentLoop;

fn help() {
    println!(
        "commands:
  <task>                    在当前会话运行一个任务
  /new <id> | /use <id>     新建/切换会话
  /fork <from> <to>         分叉会话
  /list                     列出全部会话
  /teams create <id> <name> <member...>
  /teams run <team> <task>  建任务并真实委派执行
  /teams tasks <team>       列出团队任务
  /teams msg <team> <from> <content...>
  /teams msgs <team>        列出团队消息
  /rsi round <n> <seedtask> 跑一轮 RSI 评测
  /workspace goals          列出目标
  /workspace goal add <id> <title...>
  /workspace goal done <id>
  /web fetch <url>          真实 HTTP GET
  /queue publish <channel> <json>
  /queue consume <channel>
  /code run <code>          真实 python3 执行
  /help | /quit"
    );
}

/// 取剩余参数(命令词之后的原始文本)。
fn rest(parts: &[&str], skip: usize) -> String {
    parts
        .iter()
        .skip(skip)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 真实持久化位置:当前目录下 .agent-harness/(workspace + sessions + 各存储)。
    let root = std::env::current_dir()?.join(".agent-harness");
    let workspace_root = root.join("workspace");
    let session_path = root.join("default.jsonl");
    let session_dir = root.join("sessions");
    let memory_dir = root.join("memory");
    let retrieval_dir = root.join("retrieval");
    let telemetry_dir = root.join("telemetry");

    let profile_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "profiles/dev.toml".to_string());

    let (ctx, _effects) = ah_app::boot(
        &profile_path,
        &workspace_root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
        &telemetry_dir,
    )?;
    let (agent, manager) = ah_app::agent_and_manager(&ctx)?;
    let default_session = ctx
        .service::<dyn SessionLog>(&SESSIONS)
        .ok_or("sessions seam missing")?;

    println!("agent-harness CLI (profile {profile_path})");
    println!("sessions dir: {}", session_dir.display());
    println!("type a task, or /help");
    help();

    let mut current: Option<std::sync::Arc<dyn SessionLog>> = Some(default_session);
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        print!("> ");
        std::io::stdout().flush()?;
        let Some(Ok(line)) = lines.next() else { break };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line == "/quit" || line == "/exit" {
            break;
        }
        if let Some(command) = line.strip_prefix('/') {
            let parts: Vec<&str> = command.split_whitespace().collect();
            match parts.as_slice() {
                ["help"] => help(),
                ["new", id] => {
                    current = Some(manager.create(id)?);
                    println!("switched to session {id}");
                }
                ["use", id] => {
                    current = Some(manager.open(id)?);
                    println!("switched to session {id}");
                }
                ["fork", from, to] => {
                    current = Some(manager.fork(from, to)?);
                    println!("forked {from} -> {to} and switched");
                }
                ["list"] => println!("sessions: {:?}", manager.list()),
                // ---- teams(SQLite 持久化)----
                ["teams", "create", id, name] => {
                    let teams = ctx
                        .service::<dyn ah_contracts::teams::TeamRuntime>(&TEAMS)
                        .ok_or("teams seam missing")?;
                    teams
                        .create_team(
                            ah_contracts::teams::TeamSpec {
                                id: id.to_string(),
                                name: name.to_string(),
                            },
                            vec![ah_contracts::teams::TeamMemberSpec {
                                id: "agent".to_string(),
                                name: "agent".to_string(),
                                role: "default".to_string(),
                            }],
                        )
                        .map_err(|e| e.0)?;
                    println!("team {id} created");
                }
                ["teams", "run", team, ..] => {
                    let teams = ctx
                        .service::<dyn ah_contracts::teams::TeamRuntime>(&TEAMS)
                        .ok_or("teams seam missing")?;
                    let task = rest(&parts, 3);
                    teams
                        .add_task(
                            team,
                            ah_contracts::teams::TeamTask {
                                id: task.clone(),
                                title: task.clone(),
                                status: ah_contracts::teams::TeamTaskStatus::Pending,
                                dependencies: vec![],
                                assignee: None,
                                review_votes: vec![],
                                result: None,
                            },
                        )
                        .map_err(|e| e.0)?;
                    let result = teams.run_task(team, &task).await.map_err(|e| e.0)?;
                    println!("task {task}: {:?} -> {}", result.status, result.output);
                }
                ["teams", "tasks", team] => {
                    let teams = ctx
                        .service::<dyn ah_contracts::teams::TeamRuntime>(&TEAMS)
                        .ok_or("teams seam missing")?;
                    for task in teams.tasks(team).map_err(|e| e.0)? {
                        println!(
                            "  {} [{:?}] assignee={:?}",
                            task.id, task.status, task.assignee
                        );
                    }
                }
                ["teams", "msg", team, from, ..] => {
                    let teams = ctx
                        .service::<dyn ah_contracts::teams::TeamRuntime>(&TEAMS)
                        .ok_or("teams seam missing")?;
                    let content = rest(&parts, 4);
                    teams
                        .send_message(team, from, None, &content)
                        .map_err(|e| e.0)?;
                    println!("message sent to team {team}");
                }
                ["teams", "msgs", team] => {
                    let teams = ctx
                        .service::<dyn ah_contracts::teams::TeamRuntime>(&TEAMS)
                        .ok_or("teams seam missing")?;
                    for message in teams.messages(team).map_err(|e| e.0)? {
                        println!(
                            "  {} -> {}: {}",
                            message.from,
                            message.to.as_deref().unwrap_or("*"),
                            message.content
                        );
                    }
                }
                // ---- rsi ----
                ["rsi", "round", n, ..] => {
                    let rsi = ctx
                        .service::<dyn ah_contracts::rsi::RsiRuntime>(&RSI)
                        .ok_or("rsi seam missing")?;
                    let seed = rest(&parts, 3);
                    let cases = rsi.generate_dataset(vec![seed], 3).map_err(|e| e.0)?;
                    let round = rsi
                        .evaluate_round(n.parse().unwrap_or(1), &cases, "You are a helpful agent.")
                        .await
                        .map_err(|e| e.0)?;
                    println!(
                        "rsi round {}: {}/{} passed, avg {:.2}",
                        round.round, round.passed, round.total, round.avg_score
                    );
                }
                // ---- workspace ----
                ["workspace", "goals"] => {
                    let ws = ctx
                        .service::<dyn WorkspaceService>(&WORKSPACE)
                        .ok_or("workspace seam missing")?;
                    for goal in ws.goals().map_err(|e| e.0)? {
                        println!("  {} [{:?}] {}", goal.id, goal.status, goal.title);
                    }
                }
                ["workspace", "goal", "add", id, ..] => {
                    let ws = ctx
                        .service::<dyn WorkspaceService>(&WORKSPACE)
                        .ok_or("workspace seam missing")?;
                    let title = rest(&parts, 4);
                    ws.create_goal(id, &title).map_err(|e| e.0)?;
                    println!("goal {id} added");
                }
                ["workspace", "goal", "done", id] => {
                    let ws = ctx
                        .service::<dyn WorkspaceService>(&WORKSPACE)
                        .ok_or("workspace seam missing")?;
                    ws.update_goal_status(id, GoalStatus::Done)
                        .map_err(|e| e.0)?;
                    println!("goal {id} done");
                }
                // ---- web ----
                ["web", "fetch", url] => {
                    let web = ctx
                        .service::<dyn ah_contracts::web::WebProvider>(&WEB)
                        .ok_or("web seam missing")?;
                    let result = web
                        .fetch(ah_contracts::web::WebFetchRequest {
                            url: url.to_string(),
                            timeout_ms: Some(10_000),
                        })
                        .map_err(|e| e.0)?;
                    println!(
                        "status {} body: {}",
                        result.status,
                        result.body.chars().take(200).collect::<String>()
                    );
                }
                // ---- queue ----
                ["queue", "publish", channel, json] => {
                    let queue = ctx
                        .service::<dyn ah_contracts::queue::MessageQueue>(&QUEUE)
                        .ok_or("queue seam missing")?;
                    let message = queue
                        .publish(
                            channel,
                            serde_json::from_str(json).unwrap_or(serde_json::json!(json)),
                        )
                        .map_err(|e| e.0)?;
                    println!("published seq {}", message.seq);
                }
                ["queue", "consume", channel] => {
                    let queue = ctx
                        .service::<dyn ah_contracts::queue::MessageQueue>(&QUEUE)
                        .ok_or("queue seam missing")?;
                    match queue.consume(channel).map_err(|e| e.0)? {
                        Some(message) => {
                            println!("consumed seq {}: {}", message.seq, message.payload)
                        }
                        None => println!("no messages"),
                    }
                }
                // ---- code ----
                ["code", "run", ..] => {
                    let code = ctx
                        .service::<dyn ah_contracts::code::CodeProvider>(&CODE)
                        .ok_or("code seam missing")?;
                    let body = rest(&parts, 2);
                    let result = code
                        .execute(ah_contracts::code::CodeExecRequest {
                            language: "python3".to_string(),
                            code: body,
                            timeout_ms: Some(10_000),
                        })
                        .await
                        .map_err(|e| e.0)?;
                    println!("exit {} stdout: {}", result.exit_code, result.stdout.trim());
                    if !result.stderr.trim().is_empty() {
                        println!("stderr: {}", result.stderr.trim());
                    }
                }
                _ => println!("unknown command; /help"),
            }
            continue;
        }
        // 任务:在当前会话运行。
        let Some(session) = current.clone() else {
            println!("no active session; use /new <id>");
            continue;
        };
        match run_task(&agent, &session, &line).await {
            Ok(answer) => println!("answer: {answer}"),
            Err(error) => println!("error: {error}"),
        }
    }
    Ok(())
}

/// 在指定会话运行任务(日志驱动;同一会话连续任务共享历史)。
async fn run_task(
    agent: &AgentLoop,
    session: &std::sync::Arc<dyn SessionLog>,
    task: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    agent
        .run_in_session(session.clone(), task)
        .await
        .map_err(|e| e.0.into())
}
