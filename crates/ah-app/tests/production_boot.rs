//! P0-03 production boot smoke.
//!
//! This test is opt-in because it requires real Redis and model credentials:
//! `AH_PRODUCTION_SMOKE=1 AH_ENV_FILE=$PWD/.env cargo test -p ah-app --test production_boot -- --nocapture`.

use std::path::PathBuf;

use ah_contracts::agent::{AgentRequest, AgentRunState, ApplicationRuntime};
use ah_contracts::checkpointer::CheckpointerProvider;
use ah_contracts::keys::{APPLICATION, CHECKPOINTER_PROVIDER, KV_STORE, LLM, QUEUE};
use ah_contracts::llm::ModelProvider;
use ah_contracts::queue::MessageQueue;
use ah_contracts::store::BaseKVStore;

fn paths() -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("ah-prod-boot-smoke-{}", std::process::id()));
    (
        root.clone(),
        root.join("default.jsonl"),
        root.join("sessions"),
        root.join("memory"),
        root.join("retrieval"),
        root.join("telemetry"),
    )
}

#[tokio::test]
async fn production_profile_boots_and_invokes_real_services() {
    if std::env::var("AH_PRODUCTION_SMOKE").as_deref() != Ok("1") {
        println!("skipping: set AH_PRODUCTION_SMOKE=1 to run real production smoke");
        return;
    }
    assert!(
        std::env::var_os("AH_ENV_FILE").is_some(),
        "set AH_ENV_FILE explicitly so stale process OPENAI_* values cannot override the smoke configuration"
    );

    let (root, session_path, session_dir, memory_dir, retrieval_dir, telemetry_dir) = paths();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("boot root");
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

    assert!(ctx.service::<dyn ModelProvider>(&LLM).is_some());
    let kv = ctx.service::<dyn BaseKVStore>(&KV_STORE).expect("redis kv");
    let key = format!("ah:p0-03:{}", std::process::id());
    kv.set(&key, serde_json::json!({"ok": true}))
        .expect("redis set");
    assert_eq!(kv.get(&key).expect("redis get").unwrap()["ok"], true);
    kv.delete(&key).expect("redis delete");

    let queue = ctx
        .service::<dyn MessageQueue>(&QUEUE)
        .expect("redis queue");
    let channel = format!("p0-03-smoke-{}", std::process::id());
    queue
        .publish(&channel, serde_json::json!({"ok": true}))
        .expect("redis queue publish");
    let message = queue
        .consume(&channel)
        .expect("redis queue consume")
        .expect("queued message");
    assert_eq!(message.payload["ok"], true);

    let checkpointer = ctx
        .service::<dyn CheckpointerProvider>(&CHECKPOINTER_PROVIDER)
        .expect("checkpointer provider");
    let checkpointer = checkpointer
        .create(&serde_json::json!({
            "connection": {"url": std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".to_string())}
        }))
        .expect("real redis checkpointer");
    assert!(checkpointer.graph_store().is_some());

    let application = ctx
        .service::<dyn ApplicationRuntime>(&APPLICATION)
        .expect("application seam");
    let result = application
        .invoke(AgentRequest {
            session_id: format!("p0-03-smoke-{}", std::process::id()),
            input: "Reply with exactly: p0-03 smoke ok".to_string(),
            workflow: None,
            timeout_ms: Some(30_000),
            restore_checkpoint: None,
            command: None,
            user_id: None,
            model: None,
            temperature: None,
        })
        .await
        .expect("production application invoke");
    assert_eq!(result.state, AgentRunState::Completed);
    assert!(result.answer.unwrap_or_default().contains("p0-03"));

    drop(effects);
    let _ = std::fs::remove_dir_all(root);
}
