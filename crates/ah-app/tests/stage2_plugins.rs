//! Stage 2 runtime integration: catalog/profile entries must mount, resolve,
//! execute a representative operation, and unregister through dropped Effects.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ah_contracts::a2a::A2aError;
use ah_contracts::agent_builder::AgentBuilder;
use ah_contracts::bridge_compose::BridgeCompose;
use ah_contracts::dataset_curator::{DatasetCurationConfig, DatasetCurator};
use ah_contracts::external_format::{ExternalFormat, MessageView};
use ah_contracts::inbound_render::InboundRender;
use ah_contracts::interaction_router::InteractionRouter;
use ah_contracts::keys::{
    A2A, AGENT_BUILDER, BRIDGE_COMPOSE, DATA_LOADER, DATASET_CURATOR, EXTERNAL_FORMAT,
    INBOUND_RENDER, INTERACTION_ROUTER, MODEL_ALLOCATOR, PROMPT_ATTACHMENT,
    PROMPT_ATTACHMENT_STORE, TIMEFMT,
};
use ah_contracts::model_allocator::{AllocatorStrategy, ModelAllocatorFactory, ModelPoolEntry};
use ah_contracts::prompt_attachment::{PromptAttachmentKind, PromptAttachmentStore};
use ah_contracts::timefmt::Timefmt;
use ah_hub::context::Context;
use ah_hub::plugin::DynPlugin;
use ah_plugins_data_loader::BatchPlanner;

fn test_root() -> PathBuf {
    std::env::temp_dir().join(format!("ah-stage2-app-{}", std::process::id()))
}

fn selected_plugins(root: &Path) -> Vec<DynPlugin> {
    let session_path = root.join("default.jsonl");
    let session_dir = root.join("sessions");
    let catalog = ah_app::plugin_catalog(
        root,
        &session_path,
        &session_dir,
        &root.join("memory"),
        &root.join("retrieval"),
        &root.join("telemetry"),
    );
    let names = [
        "ah-plugins-mock",
        "ah-plugins-tools",
        "ah-plugins-session-log",
        "ah-plugins-workflow",
        "ah-plugins-agentbuilder",
        "ah-plugins-a2a",
        "ah-plugins-bridge-compose",
        "ah-plugins-data-loader",
        "ah-plugins-dataset-curator",
        "ah-plugins-external-format",
        "ah-plugins-inbound-render",
        "ah-plugins-interaction-router",
        "ah-plugins-model-allocator",
        "ah-plugins-prompt-attachment",
        "ah-plugins-timefmt",
    ];
    names
        .iter()
        .map(|name| {
            catalog
                .iter()
                .find(|(candidate, _)| candidate == name)
                .unwrap_or_else(|| panic!("catalog missing {name}"))
                .1
                .clone()
        })
        .collect()
}

#[tokio::test]
async fn catalog_plugins_mount_resolve_invoke_and_unmount() {
    let root = test_root();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("test root");

    let ctx = Context::new();
    let effects = ctx
        .mount_all(selected_plugins(&root))
        .expect("stage 2 plugins mount");

    // Resolve and invoke the agent-builder seam through its injected workflow.
    let builder = ctx
        .service::<dyn AgentBuilder>(&AGENT_BUILDER)
        .expect("agent-builder");
    let design = builder
        .design("read the project and inspect the files")
        .expect("design");
    assert!(design.tools.contains(&"read_file".to_string()));
    let dsl = builder.to_dsl(&design).expect("dsl");
    assert_eq!(dsl["id"], design.name);

    let a2a = ctx
        .service::<dyn ah_plugins_a2a::A2AAdapter>(&A2A)
        .expect("a2a");
    assert_eq!(
        a2a.resolve_session_id(&serde_json::json!({"sessionId": "s2"})),
        Some("s2".into())
    );
    assert!(matches!(
        a2a.resolve_transport_protocols(&[serde_json::json!({"protocol_binding": "GRPC"})]),
        Err(A2aError(_))
    ));

    let bridge = ctx
        .service::<dyn BridgeCompose>(&BRIDGE_COMPOSE)
        .expect("bridge-compose");
    assert!(
        bridge
            .compose_bridge_inbound("alice", "hello", "done", "en", None)
            .contains("alice")
    );

    let _planner = ctx
        .service::<BatchPlanner>(&DATA_LOADER)
        .expect("data-loader");
    let mut case = BTreeMap::new();
    case.insert("case_id".to_string(), serde_json::json!("case-1"));
    case.insert("difficulty".to_string(), serde_json::json!("easy"));
    case.insert("dimension".to_string(), serde_json::json!("planning"));
    assert_eq!(BatchPlanner::plan(vec![case], 1).len(), 1);

    // Missing input is still a real invocation and must return an explicit error.
    let curator = ctx
        .service::<dyn DatasetCurator>(&DATASET_CURATOR)
        .expect("dataset-curator");
    let err = curator
        .curate(
            &DatasetCurationConfig::default(),
            "/missing/eval-ref.json",
            "/missing/output",
        )
        .expect_err("missing eval-ref must be explicit");
    assert!(!err.0.is_empty(), "curation error must be explicit");

    let inbound = ctx
        .service::<dyn InboundRender>(&INBOUND_RENDER)
        .expect("inbound-render");
    let inbound_xml =
        inbound.render_inbound("hello", "alice", "m1", "direct", "now", false, None, None);
    assert!(inbound_xml.starts_with("<team-inbound "));

    let timefmt = ctx.service::<dyn Timefmt>(&TIMEFMT).expect("timefmt");
    let external = ctx
        .service::<dyn ExternalFormat>(&EXTERNAL_FORMAT)
        .expect("external-format");
    let rendered = external.render_message(
        &MessageView {
            broadcast: false,
            timestamp: 1_700_000_000_000,
            from_member_name: "alice".to_string(),
            message_id: "m1".to_string(),
            content: "hello".to_string(),
        },
        false,
        1_700_000_001_000,
        None,
        Some("reply"),
        None,
        inbound.as_ref(),
        timefmt.as_ref(),
    );
    assert!(rendered.contains("reply-hint"));

    let router = ctx
        .service::<dyn InteractionRouter>(&INTERACTION_ROUTER)
        .expect("interaction-router");
    let payloads = router.parse_interact_str("@alice hello");
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0].target.as_deref(), Some("alice"));

    let allocator_factory = ctx
        .service::<dyn ModelAllocatorFactory>(&MODEL_ALLOCATOR)
        .expect("model-allocator");
    let allocator = allocator_factory
        .build_allocator(
            &[ModelPoolEntry::new("model-a", "provider-a")],
            AllocatorStrategy::RoundRobin,
        )
        .expect("allocator");
    assert_eq!(
        allocator
            .allocate(None)
            .expect("allocation")
            .entry
            .model_name,
        "model-a"
    );

    let attachment_api = ctx
        .service::<dyn ah_contracts::prompt_attachment::PromptAttachmentApi>(&PROMPT_ATTACHMENT)
        .expect("prompt-attachment api");
    assert_eq!(attachment_api.content_sha256(Some("abc")).len(), 64);
    let store = ctx
        .service::<dyn PromptAttachmentStore>(&PROMPT_ATTACHMENT_STORE)
        .expect("prompt-attachment store");
    let attachment = store
        .add_section(
            "session",
            "notes",
            "hello",
            PromptAttachmentKind::Text,
            "test",
            10,
            None,
            "text/plain",
            None,
        )
        .expect("add attachment");
    assert_eq!(
        store
            .get_by_id(&attachment.id, Some("session"))
            .expect("stored attachment")
            .content
            .as_deref(),
        Some("hello")
    );

    drop(effects);
    for key in [
        A2A,
        AGENT_BUILDER,
        BRIDGE_COMPOSE,
        DATA_LOADER,
        DATASET_CURATOR,
        EXTERNAL_FORMAT,
        INBOUND_RENDER,
        INTERACTION_ROUTER,
        MODEL_ALLOCATOR,
        PROMPT_ATTACHMENT,
        PROMPT_ATTACHMENT_STORE,
        TIMEFMT,
    ] {
        assert!(
            !ctx.has_service(&key),
            "{key:?} must unregister on Effect drop"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}
