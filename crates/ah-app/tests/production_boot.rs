//! Production composition E2E for P1 application/controller/workflow and P2-06 retrieval/memory backends.
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
use ah_contracts::evolving::EvolvingRuntime;
use ah_contracts::keys::{
    APPLICATION, CONTROLLER, EVOLVING, KV_STORE, LLM, QUERY_RERANK, QUEUE, RETRIEVAL, RSI, TOOLS,
};
use ah_contracts::queue::MessageQueue;
use ah_contracts::rerank::QueryReranker;
use ah_contracts::retrieval::{RetrievalHit, RetrievalProvider};
use ah_contracts::rsi::RsiRuntime;
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
    requests: Arc<Mutex<Vec<(String, String)>>>,
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

fn model_response(request_line: &str, body: &str) -> (String, String) {
    if request_line.contains(" /v1/embeddings ") {
        let vector = if body.to_lowercase().contains("pasta") {
            "[0.0,1.0]"
        } else {
            "[1.0,0.0]"
        };
        return (
            "application/json".to_string(),
            format!(r#"{{"data":[{{"embedding":{vector}}}]}}"#),
        );
    }
    if request_line.contains(" /dashscope-embedding ") {
        return (
            "application/json".to_string(),
            r#"{"output":{"embeddings":[{"embedding":[1.0,0.0],"text_index":0}]}}"#.to_string(),
        );
    }
    if request_line.contains(" /rerank ") {
        return (
            "application/json".to_string(),
            r#"{"output":{"results":[{"index":1,"relevance_score":0.9},{"index":0,"relevance_score":0.1}]}}"#
                .to_string(),
        );
    }
    if body.contains("Generate ") && body.contains("task variations") {
        return (
            "application/json".to_string(),
            r#"{"choices":[{"message":{"role":"assistant","content":"[{\"task\":\"fixture generated task\"}]"}}]}"#.to_string(),
        );
    }
    if body.contains("strict evaluator") {
        return (
            "application/json".to_string(),
            r#"{"choices":[{"message":{"role":"assistant","content":"{\"verdict\":\"pass\",\"score\":0.9,\"feedback\":\"fixture judge accepted\"}"}}]}"#.to_string(),
        );
    }
    if body.contains("You are an optimizer") {
        return (
            "application/json".to_string(),
            r#"{"choices":[{"message":{"role":"assistant","content":"[{\"target\":\"prompt\",\"suggestion\":\"fixture refinement\",\"rationale\":\"fixture feedback\",\"confidence\":0.8}]"}}]}"#.to_string(),
        );
    }
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
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_stop = stop.clone();
    let server_requests = requests.clone();
    thread::spawn(move || {
        while !server_stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let Some((request_line, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    server_requests
                        .lock()
                        .unwrap()
                        .push((request_line.clone(), body.clone()));
                    let (content_type, response_body) = model_response(&request_line, &body);
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
        requests,
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
            "OPENAI_API_KEY=fixture-key\nOPENAI_BASE_URL={}\nOPENAI_MODEL=test-model\nREDIS_URL={}\nEMBEDDING_BASE_URL={}/embeddings\nEMBEDDING_MODEL=fixture-embedding\nEMBEDDING_API_KEY=fixture-key\nDASHSCOPE_API_KEY=dashscope-fixture-key\nDASHSCOPE_EMBEDDING_ENDPOINT=http://{}/dashscope-embedding\nDASHSCOPE_EMBEDDING_MODEL=fixture-dashscope-embedding\nDASHSCOPE_RERANK_ENDPOINT=http://{}/rerank\nDASHSCOPE_RERANK_MODEL=fixture-rerank\n",
            fixture.base_url,
            std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string()),
            fixture.base_url,
            fixture.address,
            fixture.address,
        )
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
    let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("redis kv");
    let memory_key = format!("p206-memory-{}", std::process::id());
    let memory_query = format!("p206 memory {}", std::process::id());
    tools
        .invoke(
            "remember",
            serde_json::json!({"key": memory_key.as_str(), "content": memory_query.as_str()}),
        )
        .await
        .expect("production remember");
    assert_eq!(
        tools
            .invoke(
                "recall",
                serde_json::json!({"query": memory_query.as_str()}),
            )
            .await
            .expect("production recall")["count"],
        1
    );
    assert!(
        kv.get(&format!("memory:{memory_key}"))
            .expect("external memory lookup")
            .is_some(),
        "memory must be stored in the production KV backend"
    );

    let _retrieval = ctx
        .service::<dyn RetrievalProvider>(&RETRIEVAL)
        .expect("retrieval provider");
    let doc_id = format!("p206-doc-{}", std::process::id());
    tools
        .invoke(
            "ingest_knowledge",
            serde_json::json!({
                "doc_id": doc_id.as_str(),
                "text": "Rust memory retrieval document"
            }),
        )
        .await
        .expect("production ingest");
    let vector = tools
        .invoke(
            "search_knowledge",
            serde_json::json!({"query": "Rust memory", "mode": "vector", "k": 1}),
        )
        .await
        .expect("production vector search");
    assert_eq!(vector["count"], 1);
    assert_eq!(vector["hits"][0]["doc_id"], doc_id);
    assert!(
        kv.scan("retrieval-vector:")
            .expect("external vector scan")
            .iter()
            .any(|entry| entry.value["doc_id"] == doc_id),
        "vector index must be stored in the production KV backend"
    );

    let query_reranker = ctx
        .service::<dyn QueryReranker>(&QUERY_RERANK)
        .expect("query reranker");
    let reranked = query_reranker
        .rerank_query(
            "p206 rerank query",
            &[
                RetrievalHit {
                    doc_id: "first".into(),
                    chunk: "first candidate".into(),
                    score: 0.0,
                },
                RetrievalHit {
                    doc_id: "second".into(),
                    chunk: "second candidate".into(),
                    score: 0.0,
                },
            ],
            2,
        )
        .expect("production query rerank");
    assert_eq!(reranked[0].doc_id, "second");
    assert_eq!(reranked[0].score, 0.9);

    let requests = fixture.requests.lock().unwrap().clone();
    assert!(
        requests
            .iter()
            .any(|(line, _)| line.contains(" /v1/embeddings ")),
        "OpenAI-compatible embedding endpoint must be used"
    );
    assert!(
        requests
            .iter()
            .any(|(line, body)| line.contains(" /rerank ") && body.contains("p206 rerank query")),
        "query-aware reranker endpoint must receive the query"
    );
    assert!(
        !requests
            .iter()
            .any(|(line, _)| line.contains(" /dashscope-embedding ")),
        "DashScope embedding must not override EMBEDDING_BASE_URL"
    );
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
    let types: Vec<String> = {
        let chunks = sink.chunks.lock().unwrap();
        chunks
            .iter()
            .filter_map(|chunk| chunk["type"].as_str().map(str::to_string))
            .collect()
    };
    assert!(types.iter().any(|ty| ty == "workflow_delta"));
    assert!(types.iter().any(|ty| ty == "workflow_node"));
    assert_eq!(types.last().map(String::as_str), Some("workflow_final"));

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
    let tools_after_restart = ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .expect("production tools after restart");
    let recalled_after_restart = tools_after_restart
        .invoke(
            "recall",
            serde_json::json!({"query": memory_query.as_str()}),
        )
        .await
        .expect("production recall after restart");
    assert_eq!(recalled_after_restart["count"], 1);
    let vector_after_restart = tools_after_restart
        .invoke(
            "search_knowledge",
            serde_json::json!({"query": "Rust memory", "mode": "vector", "k": 1}),
        )
        .await
        .expect("production vector search after restart");
    assert_eq!(vector_after_restart["hits"][0]["doc_id"], doc_id);
    let retrieval_after_restart = ctx
        .service::<dyn RetrievalProvider>(&RETRIEVAL)
        .expect("retrieval provider after restart");
    retrieval_after_restart
        .remove(&doc_id)
        .expect("remove production vector document");
    tools_after_restart
        .invoke("forget", serde_json::json!({"key": memory_key.as_str()}))
        .await
        .expect("production forget after restart");
    drop(effects);
    std::fs::write(
        &env_file,
        format!(
            "OPENAI_API_KEY=fixture-key\nOPENAI_BASE_URL={}\nOPENAI_MODEL=test-model\nREDIS_URL={}\nEMBEDDING_BASE_URL=\nEMBEDDING_MODEL=fixture-embedding\nEMBEDDING_API_KEY=fixture-key\nDASHSCOPE_API_KEY=dashscope-fixture-key\nDASHSCOPE_EMBEDDING_ENDPOINT=http://{}/dashscope-embedding\nDASHSCOPE_EMBEDDING_MODEL=fixture-dashscope-embedding\nDASHSCOPE_RERANK_ENDPOINT=http://{}/rerank\nDASHSCOPE_RERANK_MODEL=fixture-rerank\n",
            fixture.base_url,
            std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string()),
            fixture.address,
            fixture.address,
        ),
    )
    .expect("native embedding env file");
    let native_root = root.join("p206-native");
    let native_session_path = native_root.join("default.jsonl");
    let native_session_dir = native_root.join("sessions");
    let native_memory_dir = native_root.join("memory");
    let native_retrieval_dir = native_root.join("retrieval");
    let native_telemetry_dir = native_root.join("telemetry");
    let (native_ctx, native_effects) = ah_app::boot(
        profile,
        &native_root,
        &native_session_path,
        &native_session_dir,
        &native_memory_dir,
        &native_retrieval_dir,
        &native_telemetry_dir,
    )
    .expect("native DashScope production boot");
    let native_tools = native_ctx
        .service::<dyn ToolRegistry>(&TOOLS)
        .expect("native production tools");
    let native_doc = format!("p206-native-doc-{}", std::process::id());
    native_tools
        .invoke(
            "ingest_knowledge",
            serde_json::json!({
                "doc_id": native_doc.as_str(),
                "text": "DashScope native embedding document"
            }),
        )
        .await
        .expect("native embedding ingest");
    let native_vector = native_tools
        .invoke(
            "search_knowledge",
            serde_json::json!({"query": "DashScope native", "mode": "vector", "k": 1}),
        )
        .await
        .expect("native embedding vector search");
    assert_eq!(native_vector["count"], 1);
    assert_eq!(native_vector["hits"][0]["doc_id"], native_doc);
    let native_retrieval = native_ctx
        .service::<dyn RetrievalProvider>(&RETRIEVAL)
        .expect("native retrieval provider");
    native_retrieval
        .remove(&native_doc)
        .expect("remove native vector document");
    assert!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(line, _)| line.contains(" /dashscope-embedding ")),
        "DashScope native embedding endpoint must be used when compatible endpoint is empty"
    );
    drop(native_effects);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
#[ignore = "requires a real Redis service; CI runs this test explicitly"]
async fn production_profile_exercises_p3_evolving_and_rsi() {
    let fixture = start_model_fixture();
    let root = std::env::temp_dir().join(format!("ah-prod-p3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("boot root");
    let env_file = root.join("production.env");
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string());
    std::fs::write(
        &env_file,
        format!(
            "OPENAI_API_KEY=fixture-key\nOPENAI_BASE_URL={}\nOPENAI_MODEL=test-model\nREDIS_URL={}\n",
            fixture.base_url, redis_url
        ),
    )
    .expect("production env file");
    let _env = EnvGuard::set(&[
        ("AH_ENV_FILE", env_file.display().to_string()),
        ("REDIS_URL", redis_url),
    ]);
    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/prod.toml");
    let session_path = root.join("default.jsonl");
    let session_dir = root.join("sessions");
    let memory_dir = root.join("memory");
    let retrieval_dir = root.join("retrieval");
    let telemetry_dir = root.join("telemetry");
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

    let (agent, manager) = ah_app::agent_and_manager(&ctx).expect("agent and session manager");
    let session = manager
        .create("p3-evolving")
        .expect("create evolving session");
    let result = agent
        .run_in_session(session, "produce a concise production answer")
        .await;
    assert_eq!(result.state, AgentRunState::Completed);

    let evolving = ctx
        .service::<dyn EvolvingRuntime>(&EVOLVING)
        .expect("evolving runtime");
    let evolution = evolving
        .evolve_session("produce a concise production answer", "p3-evolving")
        .await
        .expect("production evolving loop");
    assert_eq!(
        evolution.evaluation.verdict,
        ah_contracts::evolving::Verdict::Pass
    );
    assert!(
        evolution
            .evaluation
            .feedback
            .contains("fixture judge accepted")
    );
    assert!(
        evolution
            .refinements
            .iter()
            .any(|refinement| refinement.suggestion == "fixture refinement")
    );
    let experiences = evolving.load_experiences().expect("load experience");
    assert!(
        experiences
            .iter()
            .any(|experience| { experience.task == "produce a concise production answer" })
    );

    let rsi = ctx.service::<dyn RsiRuntime>(&RSI).expect("rsi runtime");
    let kv = ctx
        .service::<dyn BaseKVStore>(&KV_STORE)
        .expect("production KV");
    kv.delete("rsi:checkpoint:latest")
        .expect("clear RSI checkpoint");
    let dataset = rsi
        .generate_dataset_llm(vec!["summarize the production trace".to_string()], 1)
        .await
        .expect("LLM dataset generation");
    assert_eq!(dataset.source, "llm");
    assert_eq!(dataset.cases.len(), 1);
    let reports = rsi
        .run_rounds(
            vec!["summarize the production trace".to_string()],
            1,
            "Return a final answer and stop.",
        )
        .await
        .expect("production RSI rounds");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].total, 1);
    assert!(
        kv.get("rsi:checkpoint:latest")
            .expect("checkpoint lookup")
            .is_some()
    );
    assert!(root.join("rsi/rsi_checkpoints.jsonl").exists());

    drop(effects);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}
