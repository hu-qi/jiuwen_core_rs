//! agent-harness demo:boot 后展示全部真实能力(端到端)。

use ah_contracts::agent::AgentStep;
use ah_contracts::fs::FsProvider;
use ah_contracts::keys::{FS, LLM, SHELL, SYMPHONY, TELEMETRY, TOOLS, WORKFLOW};
use ah_contracts::llm::{ChatMessage, ChatRole, ModelProvider, ModelRequest};
use ah_contracts::shell::ShellProvider;
use ah_contracts::symphony::{Capability, Symphony};
use ah_contracts::telemetry::TelemetryProvider;
use ah_contracts::tools::ToolRegistry;
use ah_contracts::workflow::{EdgeSpec, NodeKind, NodeSpec, WorkflowEngine, WorkflowSpec};
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
    let telemetry_dir =
        std::env::temp_dir().join(format!("ah-app-telemetry-{}", std::process::id()));

    let (ctx, _effects) = ah_app::boot(
        &profile_path,
        &workspace_root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
        &telemetry_dir,
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

    // llm 流式:默认退化单块;支持流式的 provider(openai-compatible)走真实 SSE。
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel(64);
    let mut streamed = String::new();
    let stream_provider = provider.clone();
    let stream_request = ModelRequest {
        messages: vec![ChatMessage::new(ChatRole::User, "stream this")],
        ..Default::default()
    };
    let stream_task =
        tokio::spawn(async move { stream_provider.stream_chat(stream_request, stream_tx).await });
    while let Some(chunk) = stream_rx.recv().await {
        if !chunk.done {
            streamed.push_str(&chunk.content_delta);
        }
    }
    stream_task.await??;
    println!("[llm-stream] accumulated: {streamed:?}");

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
    let vector_hits = registry
        .invoke(
            "search_knowledge",
            json!({ "query": "agent framework", "mode": "vector" }),
        )
        .await?;
    println!("[retrieval-vector] hits: {}", vector_hits["count"]);

    // agent-loop(会话驱动)
    let _step_listener = ctx.on::<AgentStep>(|step| {
        println!(
            "[agent] step {}: tool_calls={} done={}",
            step.iteration, step.tool_calls, step.done
        );
    });
    let (agent, manager) = ah_app::agent_and_manager(&ctx)?;
    let task_a = manager.create("task-a").expect("create task-a");
    let result = agent
        .run_in_session(task_a.clone(), "explore the workspace")
        .await;
    println!(
        "[agent] task-a answer: {}",
        result.answer.as_deref().unwrap_or("(no answer)")
    );
    let task_b = manager.fork("task-a", "task-b").expect("fork task-b");
    let _ = agent.run_in_session(task_b.clone(), "follow up").await;
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

    // telemetry seam:真实导出(agent 步进 + 工具执行已生成 span)。
    let telemetry = ctx
        .service::<dyn TelemetryProvider>(&TELEMETRY)
        .ok_or("telemetry seam missing")?;
    let exported = telemetry.export().await?;
    println!(
        "[telemetry] exported {exported} spans -> {}/telemetry.jsonl",
        telemetry_dir.display()
    );
    println!("[telemetry] in-memory spans: {}", telemetry.spans().len());

    // symphony:能力注册 → 指纹 → 编排 → 真实执行。
    let symphony = ctx
        .service::<dyn Symphony>(&SYMPHONY)
        .ok_or("symphony seam missing")?;
    symphony
        .register_capability(Capability {
            id: "explore".to_string(),
            name: "explore workspace".to_string(),
            description: "list and read files in the workspace".to_string(),
            tags: vec!["workspace".to_string(), "files".to_string()],
            tool: Some("list_dir".to_string()),
        })
        .expect("register capability");
    let fp = symphony.fingerprint("explore").expect("fingerprint");
    println!("[symphony] fingerprint: {} ({})", fp.name, fp.input_hint);
    let plan = symphony
        .plan("list workspace files")
        .expect("orchestration plan");
    println!(
        "[symphony] plan: {} steps ({})",
        plan.steps.len(),
        plan.rationale
    );
    let executed = symphony.execute(&plan, "list workspace files").await?;
    println!(
        "[symphony] executed steps: {:?}",
        executed["steps"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    Ok(())
}
