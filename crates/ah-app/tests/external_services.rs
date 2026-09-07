use ah_contracts::queue::MessageQueue;
use ah_contracts::retrieval::{RetrievalHit, RetrievalProvider};
use ah_contracts::sandbox::SandboxProvider;
use ah_contracts::store::BaseKVStore;
use ah_plugins_queue::PulsarRestQueue;
use ah_plugins_retrieval::{EmbeddingBackend, MilvusRetrievalProvider};
use ah_plugins_sandbox::RemoteSandboxProvider;
use ah_plugins_store::{ElasticsearchKVStore, PgStore};
use serde_json::json;
use std::sync::Arc;

fn required(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("missing {name}; set AH_ENV_FILE or the test variable"))
}

struct FixedEmbedding;

impl EmbeddingBackend for FixedEmbedding {
    fn embed(&self, _text: &str) -> Result<Vec<f64>, ah_contracts::retrieval::RetrievalError> {
        Ok(vec![0.1, 0.2])
    }
}

#[test]
#[ignore = "requires a live Elasticsearch REST service"]
fn elasticsearch_live_e2e() {
    let store = ElasticsearchKVStore::new(
        required("AH_ELASTICSEARCH_URL"),
        std::env::var("AH_ELASTICSEARCH_INDEX").unwrap_or_else(|_| "agent_harness_e2e".into()),
        std::env::var("AH_ELASTICSEARCH_API_KEY").ok(),
    )
    .expect("elasticsearch client");
    let key = format!("ah-e2e:{}", std::process::id());
    store
        .set(&key, json!({"provider": "elasticsearch"}))
        .expect("set");
    assert_eq!(
        store.get(&key).expect("get").expect("value")["provider"],
        "elasticsearch"
    );
    store.delete(&key).expect("delete");
    assert!(store.get(&key).expect("get after delete").is_none());
}

#[test]
#[ignore = "requires a live PostgreSQL-wire-compatible service"]
fn gaussdb_wire_live_e2e() {
    let store = PgStore::open(&required("AH_GAUSSDB_URL")).expect("connect gaussdb wire");
    let key = format!("ah-e2e:{}", std::process::id());
    store
        .set(&key, json!({"provider": "gaussdb-wire"}))
        .expect("set");
    assert_eq!(
        store.get(&key).expect("get").expect("value")["provider"],
        "gaussdb-wire"
    );
    store.delete(&key).expect("delete");
    assert!(store.get(&key).expect("get after delete").is_none());
}

#[test]
#[ignore = "requires Milvus REST v2 and a pre-created collection"]
fn milvus_live_e2e() {
    let provider = MilvusRetrievalProvider::new(
        required("AH_MILVUS_URL"),
        std::env::var("AH_MILVUS_COLLECTION").unwrap_or_else(|_| "agent_harness_e2e".into()),
        std::env::var("AH_MILVUS_TOKEN").ok(),
        Arc::new(FixedEmbedding),
    )
    .expect("milvus client");
    let id = format!("ah-e2e-{}", std::process::id());
    provider
        .ingest(
            &id,
            "live milvus provider verification",
            json!({"e2e": true}),
        )
        .expect("insert");
    let hits: Vec<RetrievalHit> = provider.retrieve("live milvus", 1);
    assert_eq!(
        hits.first().map(|hit| hit.doc_id.as_str()),
        Some(id.as_str())
    );
}

#[test]
#[ignore = "requires a live Pulsar REST proxy"]
fn pulsar_live_e2e() {
    let queue = PulsarRestQueue::new(
        required("AH_PULSAR_URL"),
        std::env::var("AH_PULSAR_TENANT").unwrap_or_else(|_| "public".into()),
        std::env::var("AH_PULSAR_NAMESPACE").unwrap_or_else(|_| "default".into()),
        std::env::var("AH_PULSAR_SUBSCRIPTION").unwrap_or_else(|_| "agent-harness-e2e".into()),
        std::env::var("AH_PULSAR_TOKEN").ok(),
    )
    .expect("pulsar client");
    let channel = format!("agent-harness-e2e-{}", std::process::id());
    queue
        .publish(&channel, json!({"e2e": true}))
        .expect("publish");
    let message = queue.consume(&channel).expect("consume").expect("message");
    assert_eq!(message.payload["e2e"], true);
}

#[test]
#[ignore = "requires a live remote sandbox service"]
fn remote_sandbox_live_e2e() {
    let provider = RemoteSandboxProvider::new(
        required("AH_SANDBOX_REMOTE_URL"),
        std::env::var("AH_SANDBOX_REMOTE_TOKEN").ok(),
    )
    .expect("remote sandbox client");
    let policy = provider.policy().expect("policy");
    provider.set_policy(policy.clone()).expect("set policy");
    assert_eq!(provider.policy().expect("policy after set"), policy);
    assert!(provider.check_fs("src/lib.rs").expect("fs decision").allow);
    assert!(
        !provider
            .check_command("rm -rf /tmp/agent-harness")
            .expect("command decision")
            .allow
    );
}
