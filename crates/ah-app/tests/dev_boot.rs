//! Dev-profile boot smoke: the complete catalog composition must boot without
//! cloud credentials and route one request through the application seam.

use std::path::PathBuf;

mod common;

use ah_contracts::agent::{AgentRequest, AgentRunState, ApplicationRuntime};
use ah_contracts::keys::{APPLICATION, LLM};

fn paths() -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("ah-dev-boot-smoke-{}", std::process::id()));
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
async fn dev_profile_boots_without_cloud_credentials_and_invokes_application() {
    let (root, session_path, session_dir, memory_dir, retrieval_dir, telemetry_dir) = paths();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("boot root");
    let _env = common::scratch_env_file(&root);

    let profile = concat!(env!("CARGO_MANIFEST_DIR"), "/../../profiles/dev.toml");
    let (ctx, effects) = ah_app::boot(
        profile,
        &root,
        &session_path,
        &session_dir,
        &memory_dir,
        &retrieval_dir,
        &telemetry_dir,
    )
    .expect("dev profile boot");

    assert!(
        ctx.service::<dyn ah_contracts::llm::ModelProvider>(&LLM)
            .is_some()
    );
    let application = ctx
        .service::<dyn ApplicationRuntime>(&APPLICATION)
        .expect("application seam");
    let result = application
        .invoke(AgentRequest {
            session_id: "dev-boot-smoke".to_string(),
            input: "hello".to_string(),
            workflow: None,
            timeout_ms: Some(5_000),
            restore_checkpoint: None,
            command: None,
            user_id: None,
            model: None,
            temperature: None,
        })
        .await
        .expect("application invoke");
    assert_eq!(result.state, AgentRunState::Completed);
    assert_eq!(result.session_id, "dev-boot-smoke");

    drop(effects);
    assert!(!ctx.has_service(&APPLICATION));
    assert!(!ctx.has_service(&LLM));
    let _ = std::fs::remove_dir_all(root);
}
