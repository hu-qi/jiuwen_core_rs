//! Production composition E2E for P1 application, controller and workflow paths.
//!
//! The test is ignored by default because it requires a real Redis service. CI runs it
//! with `cargo test -p ah-app --test production_boot -- --ignored --nocapture`; the
//! model endpoint is a deterministic local HTTP server exercising the real OpenAI
//! compatible provider and the production profile.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use ah_contracts::agent::{AgentRequest, AgentRunState, ApplicationRuntime};
use ah_contracts::controller::{Controller, TaskExecutor, TaskStatus};
use ah_contracts::keys::{APPLICATION, CONTROLLER, KV_STORE, LLM, QUEUE, TOOLS};
use ah_contracts::queue::MessageQueue;
use ah_contracts::seam::Seam;
use ah_contracts::store::BaseKVStore;
use ah_contracts::tools::ToolRegistry;
use ah_contracts::workflow::{WorkflowError, WorkflowStreamSink};
use async_trait::async_trait;

fn paths() -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("ah-prod-p1-{}", std::process::id()));
    (
        root.clone(),
        root.join("default.jsonl"),
        root.join("sessions"),
        root.join("memory"),
        root.join("retrieval"),
        root.join("telemetry"),
    )
}

struct EnvGuard {
    values: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn set(values: &[(&'static str, String)]) -> Self {
        let previous = values
            .iter()
            .map(|(key, _)| (*key, std::env::var_os(key)))
            .collect();
        for (key, value) in values {
            // SAFETY: this ignored integration test owns its process-wide configuration.
            unsafe { std::env::set_var(key, value) };
        }
        Self { values: previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in self.values.drain(..) {
            // SAFETY: restoration runs before the test process continues with other tests.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
struct ModelFixture {
    base_url: String,
    address: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Drop for ModelFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(self.address);
    }
}

fn read_request(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > 128 * 1024 {
            return None;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    Some((
        headers.lines().next()?.to_string(),
        String::from_utf8_lossy(&bytes[header_end..header_end + content_length]).into_owned(),
    ))
}

fn model_response(body: &str) -> (String, String) {
    if body.contains("Classify the request") {
        return (
            "application/json".to_string(),
            r#"{"choices":[{"message":{"role":"assistant","content":"{\"intent_type\":\"unknown_task\",\"task_id\":null,\"task_text\":null,\"confidence\":0.1}"}}]}"#.to_string(),
        );
    }
    if body.contains("\"stream\":true") {
        let contents: &[&str] = if body.contains("reply\\nContext") {
            &["workflow ", "production", " stream"]
        } else {
            &["p1 ", "production ", "invoke ok"]
        };
        let chunks = contents
            .iter()
            .map(|content| {
                serde_json::json!({
                    "choices": [{"delta": {"content": content}}]
                })
            })
            .map(|chunk| format!("data: {chunk}\r\n\r\n"))
            .collect::<String>();
        return (
            "text/event-stream".to_string(),
            format!("{chunks}data: [DONE]\r\n\r\n"),
        );
    }
    (
        "application/json".to_string(),
        r#"{"choices":[{"message":{"role":"assistant","content":"p1 production invoke ok"}}]}"#
            .to_string(),
    )
}
fn start_model_fixture() -> ModelFixture {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind model fixture");
    let address = listener.local_addr().expect("model fixture address");
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    thread::spawn(move || {
        while !server_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let Some((_request_line, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    let (content_type, response_body) = model_response(&body);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{response_body}",
                        response_body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(_) if !server_stop.load(Ordering::Acquire) => continue,
                Err(_) => break,
            }
        }
    });
    ModelFixture {
        base_url: format!("http://{address}/v1"),
        address,
        stop,
    }
}

#[derive(Default)]
struct RecordingSink {
    chunks: Mutex<Vec<serde_json::Value>>,
    closed: AtomicBool,
}

impl Seam for RecordingSink {}

#[async_trait]
impl WorkflowStreamSink for RecordingSink {
    async fn emit(&self, chunk: serde_json::Value) -> Result<(), WorkflowError> {
        self.chunks.lock().unwrap().push(chunk);
        Ok(())
    }

    async fn close(&self) -> Result<(), WorkflowError> {
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

struct ProductionExecutor;

#[async_trait]
impl TaskExecutor for ProductionExecutor {
    fn task_type(&self) -> &'static str {
        "agent"
    }

    async fn execute(
        &self,
        task: &ah_contracts::controller::Task,
    ) -> Result<String, ah_contracts::controller::ControllerError> {
        Ok(format!("executed {}", task.task_id))
    }
}

#[tokio::test]
#[ignore = "requires a real Redis service; CI runs this test explicitly"]
async fn production_profile_exercises_p1_application_controller_workflow() {
    let fixture = start_model_fixture();
    let (root, session_path, session_dir, memory_dir, retrieval_dir, telemetry_dir) = paths();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("boot root");
    let env_file = root.join("production.env");
    std::fs::write(
        &env_file,
        format!(
            "OPENAI_API_KEY=fixture-key\nOPENAI_BASE_URL={}\nOPENAI_MODEL=test-model\nREDIS_URL={}\n",
            fixture.base_url,
            std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())
        ),
    )
    .expect("production env file");
    let _env = EnvGuard::set(&[
        ("AH_ENV_FILE", env_file.display().to_string()),
        (
            "REDIS_URL",
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string()),
        ),
    ]);
    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/prod.toml");

    let (ctx, effects) = ah_app::boot(
        profile,
        &root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
        &telemetry_dir,
    )
    .expect("production profile boot");
    assert!(
        ctx.service::<dyn ah_contracts::llm::ModelProvider>(&LLM)
            .is_some()
    );
    let tools = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .expect("production tools");
    for name in [
        "cron",
        "edit",
        "forget",
        "glob",
        "grep",
        "read_file",
        "recall",
        "remember",
        "todo",
        "write_file",
    ] {
        assert!(
            tools.names().contains(&name.to_string()),
            "production profile missing tool {name}"
        );
    }
    tools
        .invoke(
            "write_file",
            serde_json::json!({"path": "tools/tool-smoke.txt", "content": "alpha alpha"}),
        )
        .await
        .expect("production write_file");
    assert_eq!(
        tools
            .invoke(
                "read_file",
                serde_json::json!({"path": "tools/tool-smoke.txt"})
            )
            .await
            .expect("production read_file")["content"],
        "alpha alpha"
    );
    assert_eq!(
        tools
            .invoke(
                "edit",
                serde_json::json!({"path": "tools/tool-smoke.txt", "old_string": "alpha", "new_string": "beta"}),
            )
            .await
            .expect("production edit")["replaced"],
        1
    );
    assert!(
        tools
            .invoke("glob", serde_json::json!({"pattern": "*.txt"}))
            .await
            .expect("production glob")["matches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "tools/tool-smoke.txt")
    );
    assert_eq!(
        tools
            .invoke("grep", serde_json::json!({"pattern": "beta"}))
            .await
            .expect("production grep")["match_count"],
        1
    );
    assert_eq!(
        tools
            .invoke(
                "todo",
                serde_json::json!({"action": "add", "session_id": "production", "content": "tool smoke"}),
            )
            .await
            .expect("production todo")["count"],
        1
    );
    assert_eq!(
        tools
            .invoke(
                "cron",
                serde_json::json!({"action": "add", "command": "echo tool-smoke", "schedule": "*/5 * * * *"}),
            )
            .await
            .expect("production cron")["count"],
        1
    );
    tools
        .invoke(
            "remember",
            serde_json::json!({"key": "production-tool", "content": "tool memory"}),
        )
        .await
        .expect("production remember");
    assert_eq!(
        tools
            .invoke("recall", serde_json::json!({"query": "tool memory"}))
            .await
            .expect("production recall")["count"],
        1
    );
    tools
        .invoke("forget", serde_json::json!({"key": "production-tool"}))
        .await
        .expect("production forget");

    let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("redis kv");
    let key = format!("ah:p1:{}", std::process::id());
    kv.set(&key, serde_json::json!({"ok": true}))
        .expect("redis set");
    assert_eq!(kv.get(&key).expect("redis get").unwrap()["ok"], true);
    kv.delete(&key).expect("redis delete");
    let queue = ctx
        .service::<dyn MessageQueue>(&QUEUE)
        .expect("redis queue");
    let channel = format!("p1-smoke-{}", std::process::id());
    queue
        .publish(&channel, serde_json::json!({"ok": true}))
        .expect("queue publish");
    assert_eq!(
        queue
            .consume(&channel)
            .expect("queue consume")
            .unwrap()
            .payload["ok"],
        true
    );

    let application = ctx
        .service::<dyn ApplicationRuntime>(&APPLICATION)
        .expect("application seam");
    let invoke = application
        .invoke(AgentRequest {
            session_id: "p1-application".into(),
            input: "answer from production fixture".into(),
            workflow: None,
            timeout_ms: Some(30_000),
            restore_checkpoint: None,
            command: None,
            user_id: Some("p1-user".into()),
            model: None,
            temperature: Some(0.0),
        })
        .await
        .expect("production application invoke");
    assert_eq!(invoke.state, AgentRunState::Completed);
    assert_eq!(invoke.answer.as_deref(), Some("p1 production invoke ok"));

    let controller = ctx
        .service::<dyn Controller>(&CONTROLLER)
        .expect("controller seam");
    let executor_effect = controller.register_executor(Arc::new(ProductionExecutor));
    let create = application
        .invoke(AgentRequest {
            session_id: "p1-controller".into(),
            input: "create production task".into(),
            workflow: None,
            timeout_ms: None,
            restore_checkpoint: None,
            command: Some(serde_json::json!({
                "intent_type": "create_task",
                "task_id": "p1-task",
                "task_text": "run production controller task",
                "task_type": "agent",
                "priority": 1
            })),
            user_id: None,
            model: None,
            temperature: None,
        })
        .await
        .expect("create production task");
    assert_eq!(create.state, AgentRunState::Completed);
    assert_eq!(
        controller.get_task("p1-task").unwrap().status,
        TaskStatus::Submitted
    );
    let outcomes = controller.run_pending(None).await;
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].1.as_deref(), Ok("executed p1-task"));
    assert_eq!(
        controller.get_task("p1-task").unwrap().status,
        TaskStatus::Completed
    );
    drop(executor_effect);

    let sink = Arc::new(RecordingSink::default());
    let stream = application
        .stream(
            AgentRequest {
                session_id: "p1-workflow".into(),
                input: "stream this workflow".into(),
                workflow: Some(serde_json::json!({
                    "id": "p1-workflow",
                    "nodes": [
                        {"id": "start", "kind": "start", "config": {}},
                        {"id": "llm", "kind": "llm", "config": {"prompt": "reply"}},
                        {"id": "end", "kind": "end", "config": {}}
                    ],
                    "edges": [
                        {"from": "start", "to": "llm"},
                        {"from": "llm", "to": "end"}
                    ]
                })),
                timeout_ms: Some(30_000),
                restore_checkpoint: None,
                command: None,
                user_id: Some("p1-user".into()),
                model: None,
                temperature: Some(0.0),
            },
            sink.clone(),
        )
        .await
        .expect("production workflow stream");
    assert_eq!(stream.state, AgentRunState::Completed);
    assert_eq!(
        stream.answer.as_deref(),
        Some("{\"content\":\"workflow production stream\"}")
    );
    assert!(sink.closed.load(Ordering::Acquire));
    let chunks = sink.chunks.lock().unwrap();
    let types: Vec<_> = chunks
        .iter()
        .filter_map(|chunk| chunk["type"].as_str())
        .collect();
    assert!(types.contains(&"workflow_delta"));
    assert!(types.contains(&"workflow_node"));
    assert_eq!(types.last(), Some(&"workflow_final"));
    drop(chunks);

    drop(effects);
    let (ctx, effects) = ah_app::boot(
        profile,
        &root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
        &telemetry_dir,
    )
    .expect("production profile restart");
    let restored_controller = ctx
        .service::<dyn Controller>(&CONTROLLER)
        .expect("restored controller");
    assert_eq!(
        restored_controller.get_task("p1-task").unwrap().status,
        TaskStatus::Completed
    );
    drop(effects);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}
