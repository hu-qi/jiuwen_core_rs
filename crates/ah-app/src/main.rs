//! agent-harness demo:boot 后展示全部真实能力(端到端)。

use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{FS, LLM, SHELL, TOOLS, WORKFLOW};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::shell::ShellProvider;
use ah_contracts::tools::ToolRegistry;
use ah_contracts::workflow::{EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowSpec};
use ah_plugins_agent_loop::AgentStep;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let profile_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "profiles/dev.toml".to_string());

    // 真实临时 workspace 与会话。
    let workspace_root = std::env::temp_dir().join(format!("ah-app-{}", std::process::id()));
    let session_path =
        std::env::temp_dir().join(format!("ah-app-session-{}.jsonl", std::process::id()));
    let session_dir = std::env::temp_dir().join(format!("ah-app-sessions-{}", std::process::id()));
    let memory_dir = std::env::temp_dir().join(format!("ah-app-memory-{}", std::process::id()));
    let retrieval_dir =
        std::env::temp_dir().join(format!("ah-app-retrieval-{}", std::process::id()));

    let (ctx, _effects) = ah_app::boot(
        &profile_path,
        &workspace_root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
    )?;
    println!("[boot] mounted services: {:?}", ctx.service_keys());

    // llm seam
    let provider = ctx
        .service::<dyn ModelProvider>(&LLM)
        .ok_or("llm seam missing")?;
    let response = provider
        .chat(ModelRequest {
            messages: vec![ChatMessage::new(ChatRole::User, "hello from ah-app")],
            ..Default::default()
        })
        .await?;
    println!("[llm] {}: {}", provider.name(), response.content);

    // fs seam
    let fs = ctx
        .service::<dyn FsProvider>(&FS)
        .ok_or("fs seam missing")?;
    fs.write("notes/plan.md", b"real file written by ah-app")?;
    println!("[fs] wrote notes/plan.md; entries: {:?}", fs.list("notes")?);

    // shell seam
    let shell = ctx
        .service::<dyn ShellProvider>(&SHELL)
        .ok_or("shell seam missing")?;
    let output = shell
        .run(
            "sh",
            &["-c".to_string(), "echo real-process-output".to_string()],
            std::time::Duration::from_secs(10),
        )
        .await?;
    println!(
        "[shell] exit={} stdout={}",
        output.exit_code,
        output.stdout.trim()
    );

    // tools seam + rails
    let registry = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .ok_or("tools seam missing")?;
    let _ = registry
        .invoke(
            "write_file",
            json!({ "path": "via-tool.txt", "content": "written via real tool" }),
        )
        .await?;
    let read = registry
        .invoke("read_file", json!({ "path": "via-tool.txt" }))
        .await?;
    println!(
        "[tools] write_file -> read_file roundtrip: {}",
        read["content"]
    );
    match registry
        .invoke("run_shell", json!({ "command": "rm -rf /" }))
        .await
    {
        Ok(_) => println!("[tools] WARNING: dangerous command was NOT blocked!"),
        Err(error) => println!("[tools] run_shell rm -rf / -> blocked: {error}"),
    }

    // memory seam + 工具:remember -> recall(真实持久化)。
    let _ = registry
        .invoke(
            "remember",
            json!({ "key": "app-note", "content": "agent-harness demo ran", "tags": ["demo"] }),
        )
        .await?;
    let recalled = registry
        .invoke("recall", json!({ "query": "demo ran" }))
        .await?;
    println!("[memory] recall count: {}", recalled["count"]);

    // retrieval seam + 工具:ingest_knowledge -> search_knowledge(真实检索)。
    let _ = registry
        .invoke(
            "ingest_knowledge",
            json!({ "doc_id": "openjiuwen", "text": "OpenJiuwen is an agent framework with plugins." }),
        )
        .await?;
    let hits = registry
        .invoke("search_knowledge", json!({ "query": "agent framework" }))
        .await?;
    println!("[retrieval] hits: {}", hits["count"]);

    // agent-loop(会话驱动)
    let _step_listener = ctx.on::<AgentStep>(|step| {
        println!(
            "[agent] step {}: tool_calls={} done={}",
            step.iteration, step.tool_calls, step.done
        );
    });
    let (agent, manager) = ah_app::agent_and_manager(&ctx)?;
    let task_a = manager.create("task-a").expect("create task-a");
    let answer = agent
        .run_in_session(task_a.clone(), "explore the workspace")
        .await?;
    println!("[agent] task-a answer: {answer}");
    let task_b = manager.fork("task-a", "task-b").expect("fork task-b");
    let _ = agent.run_in_session(task_b.clone(), "follow up").await?;
    println!(
        "[session] task-b events after resume: {} (task-a unchanged: {})",
        task_b.events().len(),
        task_a.events().len()
    );
    println!("[session] sessions: {:?}", manager.list());

    // workflow
    let engine = ctx
        .service::<dyn WorkflowEngine>(&WORKFLOW)
        .ok_or("workflow seam missing")?;
    let spec = WorkflowSpec {
        id: "demo-pipeline".to_string(),
        nodes: vec![
            NodeSpec {
                id: "start".into(),
                kind: NodeKind::Start,
                config: json!({}),
            },
            NodeSpec {
                id: "write".into(),
                kind: NodeKind::Tool,
                config: json!({ "tool": "write_file", "args": { "path": "computed.txt", "content": "42" } }),
            },
            NodeSpec {
                id: "llm".into(),
                kind: NodeKind::Llm,
                config: json!({ "prompt": "summarize" }),
            },
            NodeSpec {
                id: "end".into(),
                kind: NodeKind::End,
                config: json!({}),
            },
        ],
        edges: vec![
            EdgeSpec {
                from: "start".into(),
                to: "write".into(),
                condition: None,
            },
            EdgeSpec {
                from: "write".into(),
                to: "llm".into(),
                condition: None,
            },
            EdgeSpec {
                from: "llm".into(),
                to: "end".into(),
                condition: None,
            },
        ],
    };
    let wf = engine.run(&spec, json!({})).await.expect("workflow run");
    println!("[workflow] executed: {}", wf.executed.join(" -> "));
    println!(
        "[workflow] real side effect: computed.txt exists = {}",
        ctx.service::<dyn FsProvider>(&FS)
            .map(|f| f.exists("computed.txt"))
            .unwrap_or(false)
    );

    Ok(())
}
