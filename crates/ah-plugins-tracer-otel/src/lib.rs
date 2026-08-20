//! # ah-plugins-tracer-otel
//!
//! 真实 OTel 追踪扩展(1:1 对齐 `extensions/tracer_otel/` 的确定性部分):
//! - `OtelTracerConfig` 校验(sample_rate ∈ [0,1],对齐 config.py);
//! - `resolve_exporter`(exporter_type console/otlp + protocol grpc/http +
//!   http endpoint 补 `/v1/traces`,对齐 setup.py);
//! - `redact` / `truncate` / `hash_value`(sha256: + 16 hex,对齐 redaction.py);
//! - semconv 属性键常量 + span managers + 属性映射/父上下文/组件分类
//!   (对齐 span_manager.py / handler.py 确定性部分)。
//!
//! 纯函数、无 IO、无 LLM;委托契约层实现。真实 OTel tracer/导出器由宿主
//! 按 `ExporterPlan` 构建并注入。

use std::sync::Arc;

use ah_contracts::keys::OTEL_TRACER;
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_contracts::tracer_otel::{
    ExporterPlan, OtelTracer, OtelTracerConfig, OtelTracerError, redact, resolve_exporter,
    validate_sample_rate,
};
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::Value;

/// 纯函数 OTel 追踪门面实现:委托契约层。
pub struct OtelTracerImpl;

impl Seam for OtelTracerImpl {}

impl OtelTracer for OtelTracerImpl {
    fn validate(&self, config: &OtelTracerConfig) -> Result<(), OtelTracerError> {
        validate_sample_rate(config.sample_rate)
    }

    fn resolve_exporter(&self, config: &OtelTracerConfig) -> Result<ExporterPlan, OtelTracerError> {
        resolve_exporter(config)
    }

    fn redact(
        &self,
        value: Option<&Value>,
        config: &OtelTracerConfig,
        field: Option<&str>,
    ) -> String {
        redact(value, config, field)
    }
}

/// tracer-otel 插件:注册 `otel-tracer` seam。
pub struct OtelTracerPlugin;

impl Plugin for OtelTracerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-tracer-otel"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![OTEL_TRACER]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let tracer: Arc<dyn OtelTracer> = Arc::new(OtelTracerImpl);
        Ok(vec![ctx.register(OTEL_TRACER, tracer)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::OTEL_TRACER;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn build_ctx() -> (Context, Vec<Effect>) {
        let ctx = Context::new();
        let plugins: Vec<DynPlugin> = vec![Arc::new(OtelTracerPlugin)];
        let effects = ctx.mount_all(plugins).expect("mount");
        (ctx, effects)
    }

    #[test]
    fn seam_registered_and_validates() {
        let (ctx, effects) = build_ctx();
        let tracer = ctx
            .service::<dyn OtelTracer>(&OTEL_TRACER)
            .expect("otel-tracer");
        let mut config = OtelTracerConfig::default();
        assert!(tracer.validate(&config).is_ok());
        config.sample_rate = 1.5;
        assert!(tracer.validate(&config).is_err());
        drop(effects);
    }

    #[test]
    fn exporter_and_redact_via_seam() {
        let (ctx, effects) = build_ctx();
        let tracer = ctx
            .service::<dyn OtelTracer>(&OTEL_TRACER)
            .expect("otel-tracer");
        let config = OtelTracerConfig {
            exporter_type: "console".to_string(),
            ..Default::default()
        };
        assert_eq!(
            tracer.resolve_exporter(&config).expect("console"),
            ExporterPlan::Console
        );
        let config = OtelTracerConfig {
            exporter_type: "otlp".to_string(),
            protocol: "http".to_string(),
            exporter_endpoint: Some("http://c:4318".to_string()),
            ..Default::default()
        };
        let plan = tracer.resolve_exporter(&config).expect("otlp");
        assert!(format!("{plan:?}").contains("http://c:4318/v1/traces"));

        // 脱敏:启用 → 哈希;禁用 → 原文。
        assert!(
            tracer
                .redact(Some(&json!("secret")), &config, None)
                .starts_with("sha256:")
        );
        let config = OtelTracerConfig {
            redaction_enabled: false,
            ..Default::default()
        };
        assert_eq!(tracer.redact(Some(&json!("hello")), &config, None), "hello");
        drop(effects);
    }
}
