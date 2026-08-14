//! agent-harness 交互 CLI:输入任务跑 agent,会话可新建/切换/分叉。
//!
//! 会话真实持久化在 ~/.agent-harness/sessions 下;agent 执行真实
//! (dev profile 用 mock 模型桩,prod 用真实模型)。

use std::io::{BufRead, Write};

use ah_contracts::keys::SESSIONS;
use ah_contracts::session::SessionLog;
use ah_plugins_agent_loop::AgentLoop;

fn help() {
    println!(
        "commands:
  <task>            在当前会话运行一个任务
  /new <id>         新建(或打开)会话
  /use <id>         切换会话
  /fork <from> <to> 分叉会话
  /list             列出全部会话
  /help             帮助
  /quit             退出"
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 真实持久化位置:当前目录下 .agent-harness/(workspace + sessions)。
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
