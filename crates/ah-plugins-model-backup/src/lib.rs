use std::sync::Arc;

use ah_contracts::effect::Effect;
use ah_contracts::keys::{MODEL_BACKUP, MODEL_BACKUP_POLICY, MODEL_PROVIDER_CATALOG};
use ah_contracts::llm::{ModelChunk, ModelProvider, ModelRequest, ModelResponse};
use ah_contracts::model_backup::{
    ModelBackup, ModelBackupError, ModelBackupPolicy, ModelBackupPolicyProvider,
};
use ah_contracts::model_catalog::ModelProviderCatalog;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use async_trait::async_trait;
use tokio::sync::mpsc;

pub struct OrderedModelBackup {
    backups: Vec<Arc<dyn ModelProvider>>,
}

impl Seam for OrderedModelBackup {}

#[async_trait]
impl ModelBackup for OrderedModelBackup {
    async fn chat(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
    ) -> Result<ModelResponse, ModelBackupError> {
        self.chat_with_policy(primary, request, ModelBackupPolicy::default())
            .await
    }

    async fn chat_with_policy(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
        policy: ModelBackupPolicy,
    ) -> Result<ModelResponse, ModelBackupError> {
        let mut failures = Vec::new();
        let models: Vec<&dyn ModelProvider> = std::iter::once(primary)
            .chain(self.backups.iter().map(|model| model.as_ref()))
            .collect();
        for model in models {
            for attempt in 0..=policy.retries_per_model {
                let result = if let Some(timeout_ms) = policy.attempt_timeout_ms {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        model.chat(request.clone()),
                    )
                    .await
                    .map_err(|_| format!("{}: timeout", model.name()))
                    .and_then(|result| result.map_err(|error| format!("{}: {error}", model.name())))
                } else {
                    model
                        .chat(request.clone())
                        .await
                        .map_err(|error| format!("{}: {error}", model.name()))
                };
                match result {
                    Ok(response) => return Ok(response),
                    Err(error) => failures.push(format!("{error} (attempt {})", attempt + 1)),
                }
            }
        }
        Err(ModelBackupError(format!(
            "all models failed: {}",
            failures.join("; ")
        )))
    }

    async fn stream_chat_with_policy(
        &self,
        primary: &dyn ModelProvider,
        request: ModelRequest,
        sink: mpsc::Sender<ModelChunk>,
        policy: ModelBackupPolicy,
    ) -> Result<(), ModelBackupError> {
        let models: Vec<&dyn ModelProvider> = std::iter::once(primary)
            .chain(self.backups.iter().map(|model| model.as_ref()))
            .collect();
        let mut failures = Vec::new();
        for model in models {
            for attempt in 0..=policy.retries_per_model {
                let (tx, mut rx) = mpsc::channel(32);
                let result = if let Some(timeout_ms) = policy.attempt_timeout_ms {
                    tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        model.stream_chat(request.clone(), tx),
                    )
                    .await
                    .map_err(|_| format!("{}: timeout", model.name()))
                    .and_then(|result| result.map_err(|error| format!("{}: {error}", model.name())))
                } else {
                    model
                        .stream_chat(request.clone(), tx)
                        .await
                        .map_err(|error| format!("{}: {error}", model.name()))
                };
                if result.is_ok() {
                    while let Some(chunk) = rx.recv().await {
                        if sink.send(chunk).await.is_err() {
                            return Err(ModelBackupError("stream sink closed".into()));
                        }
                    }
                    return Ok(());
                }
                failures.push(format!("{} (attempt {})", result.unwrap_err(), attempt + 1));
            }
        }
        Err(ModelBackupError(format!(
            "all streaming models failed: {}",
            failures.join("; ")
        )))
    }

    fn models(&self) -> Vec<String> {
        self.backups
            .iter()
            .map(|model| model.name().to_string())
            .collect()
    }
}

pub struct StaticModelProviderCatalog {
    providers: std::collections::HashMap<String, Arc<dyn ModelProvider>>,
}

impl StaticModelProviderCatalog {
    pub fn new(
        providers: Vec<Arc<dyn ModelProvider>>,
    ) -> Result<Self, ah_contracts::model_catalog::ModelCatalogError> {
        let mut map = std::collections::HashMap::new();
        for provider in providers {
            let name = provider.name().trim();
            if name.is_empty() || map.insert(name.to_string(), provider).is_some() {
                return Err(ah_contracts::model_catalog::ModelCatalogError::new(
                    "duplicate or empty model provider name",
                ));
            }
        }
        Ok(Self { providers: map })
    }
}

impl Seam for StaticModelProviderCatalog {}
impl ModelProviderCatalog for StaticModelProviderCatalog {
    fn resolve(
        &self,
        name: &str,
    ) -> Result<Arc<dyn ModelProvider>, ah_contracts::model_catalog::ModelCatalogError> {
        self.providers.get(name).cloned().ok_or_else(|| {
            ah_contracts::model_catalog::ModelCatalogError::new(format!(
                "unknown model provider: {name}"
            ))
        })
    }
    fn names(&self) -> Vec<String> {
        let mut names: Vec<_> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }
}

pub struct StaticModelBackupPolicy(pub ModelBackupPolicy);
impl Seam for StaticModelBackupPolicy {}
impl ModelBackupPolicyProvider for StaticModelBackupPolicy {
    fn policy(&self) -> ModelBackupPolicy {
        self.0
    }
}

pub struct ModelBackupPolicyPlugin {
    pub policy: ModelBackupPolicy,
}
impl Plugin for ModelBackupPolicyPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-model-backup-policy"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![MODEL_BACKUP_POLICY]
    }
    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        Ok(vec![ctx.register(
            MODEL_BACKUP_POLICY,
            Arc::new(StaticModelBackupPolicy(self.policy)),
        )])
    }
}

pub struct ModelBackupPlugin {
    backups: Vec<Arc<dyn ModelProvider>>,
    provider_names: Vec<String>,
}

impl ModelBackupPlugin {
    pub fn new(backups: Vec<Arc<dyn ModelProvider>>) -> Self {
        Self {
            backups,
            provider_names: Vec::new(),
        }
    }

    pub fn with_provider_names(mut self, names: Vec<String>) -> Self {
        self.provider_names = names;
        self
    }
}

impl Plugin for ModelBackupPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-model-backup"
    }
    fn provides(&self) -> Vec<ServiceKey> {
        vec![MODEL_BACKUP, MODEL_BACKUP_POLICY]
    }
    fn inject(&self) -> Vec<ServiceKey> {
        if self.provider_names.is_empty() {
            vec![]
        } else {
            vec![MODEL_PROVIDER_CATALOG]
        }
    }
    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let mut backups = self.backups.clone();
        if !self.provider_names.is_empty() {
            let catalog = ctx
                .service::<dyn ModelProviderCatalog>(&MODEL_PROVIDER_CATALOG)
                .ok_or_else(|| PluginError::Apply {
                    plugin: self.name(),
                    message: "model provider catalog is required when provider_names is configured"
                        .into(),
                })?;
            for name in &self.provider_names {
                backups.push(catalog.resolve(name).map_err(|e| PluginError::Apply {
                    plugin: self.name(),
                    message: e.to_string(),
                })?);
            }
        }
        let provider: Arc<dyn ModelBackup> = Arc::new(OrderedModelBackup { backups });
        let policy: Arc<dyn ModelBackupPolicyProvider> =
            Arc::new(StaticModelBackupPolicy(ModelBackupPolicy::default()));
        Ok(vec![
            ctx.register(MODEL_BACKUP, provider),
            ctx.register(MODEL_BACKUP_POLICY, policy),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::llm::{ModelError, ModelRequest};
    use ah_contracts::model_catalog::{ModelCatalogError, ModelProviderCatalog};

    struct Failing;
    impl Seam for Failing {}
    #[async_trait]
    impl ModelProvider for Failing {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn chat(&self, _: ModelRequest) -> Result<ModelResponse, ModelError> {
            Err(ModelError("down".into()))
        }
    }

    struct Working;
    impl Seam for Working {}
    #[async_trait]
    impl ModelProvider for Working {
        fn name(&self) -> &'static str {
            "working"
        }
        async fn chat(&self, _: ModelRequest) -> Result<ModelResponse, ModelError> {
            Ok(ModelResponse {
                content: "ok".into(),
                tool_calls: vec![],
                reasoning_content: None,
            })
        }
    }

    struct Slow;
    impl Seam for Slow {}
    #[async_trait]
    impl ModelProvider for Slow {
        fn name(&self) -> &'static str {
            "slow"
        }
        async fn chat(&self, _: ModelRequest) -> Result<ModelResponse, ModelError> {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            Ok(ModelResponse {
                content: "late".into(),
                tool_calls: vec![],
                reasoning_content: None,
            })
        }
    }

    #[tokio::test]
    async fn streaming_sink_close_is_explicit_failure() {
        let backup = OrderedModelBackup {
            backups: vec![Arc::new(Working)],
        };
        let (sink, received) = mpsc::channel(1);
        drop(received);
        let error = backup
            .stream_chat_with_policy(
                &Working,
                ModelRequest::default(),
                sink,
                ModelBackupPolicy::default(),
            )
            .await
            .unwrap_err();
        assert!(error.0.contains("stream sink closed"));
    }

    #[tokio::test]
    async fn streaming_timeout_retries_before_next_model() {
        let backup = OrderedModelBackup {
            backups: vec![Arc::new(Working)],
        };
        let (sink, mut received) = mpsc::channel(4);
        backup
            .stream_chat_with_policy(
                &Slow,
                ModelRequest::default(),
                sink,
                ModelBackupPolicy {
                    attempt_timeout_ms: Some(1),
                    retries_per_model: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(received.recv().await.unwrap().content_delta, "ok");
    }

    #[allow(dead_code)]
    async fn streaming_fallback_forwards_backup_chunks() {
        let backup = OrderedModelBackup {
            backups: vec![Arc::new(Working)],
        };
        let (sink, mut received) = mpsc::channel(4);
        backup
            .stream_chat_with_policy(
                &Failing,
                ModelRequest::default(),
                sink,
                ModelBackupPolicy::default(),
            )
            .await
            .unwrap();
        let chunk = received.recv().await.unwrap();
        assert!(chunk.done);
        assert_eq!(chunk.content_delta, "ok");
    }

    #[tokio::test]
    async fn timeout_moves_to_next_model_and_retries_are_bounded() {
        let backup = OrderedModelBackup {
            backups: vec![Arc::new(Working)],
        };
        let response = backup
            .chat_with_policy(
                &Slow,
                ModelRequest::default(),
                ModelBackupPolicy {
                    attempt_timeout_ms: Some(1),
                    retries_per_model: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.content, "ok");
        let exhausted = OrderedModelBackup {
            backups: vec![Arc::new(Failing)],
        };
        let error = exhausted
            .chat_with_policy(
                &Failing,
                ModelRequest::default(),
                ModelBackupPolicy {
                    attempt_timeout_ms: None,
                    retries_per_model: 2,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.0.matches("attempt").count(), 6);
    }

    struct Catalog;
    impl Seam for Catalog {}
    impl ModelProviderCatalog for Catalog {
        fn resolve(&self, name: &str) -> Result<Arc<dyn ModelProvider>, ModelCatalogError> {
            if name == "working" {
                Ok(Arc::new(Working))
            } else {
                Err(ModelCatalogError::new(format!("unknown provider: {name}")))
            }
        }
        fn names(&self) -> Vec<String> {
            vec!["working".into()]
        }
    }

    #[test]
    fn static_catalog_validates_and_sorts_provider_names() {
        let catalog = StaticModelProviderCatalog::new(vec![Arc::new(Working)]).unwrap();
        assert_eq!(catalog.names(), vec!["working"]);
        assert!(catalog.resolve("missing").is_err());
        assert!(
            StaticModelProviderCatalog::new(vec![Arc::new(Working), Arc::new(Working)]).is_err()
        );
    }

    #[test]
    fn configured_provider_names_require_and_resolve_catalog() {
        let plugin = ModelBackupPlugin::new(Vec::new()).with_provider_names(vec!["working".into()]);
        let missing = plugin.apply(&ah_hub::context::Context::new()).unwrap_err();
        assert!(missing.to_string().contains("catalog is required"));
        let ctx = ah_hub::context::Context::new();
        let _catalog_effect = ctx.register(
            MODEL_PROVIDER_CATALOG,
            Arc::new(Catalog) as Arc<dyn ModelProviderCatalog>,
        );
        let effects = plugin.apply(&ctx).expect("catalog resolution");
        assert_eq!(effects.len(), 1);
    }

    #[tokio::test]
    async fn falls_back_in_order_and_reports_all_failures() {
        let backup = OrderedModelBackup {
            backups: vec![Arc::new(Working)],
        };
        let response = backup
            .chat(&Failing, ModelRequest::default())
            .await
            .unwrap();
        assert_eq!(response.content, "ok");
        let exhausted = OrderedModelBackup {
            backups: vec![Arc::new(Failing)],
        };
        let error = exhausted
            .chat(&Failing, ModelRequest::default())
            .await
            .unwrap_err();
        assert!(error.0.contains("all models failed"));
    }
}
