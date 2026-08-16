//! # ah-plugins-model-catalog
//!
//! Real OpenAI-account model catalog (aligned with openjiuwen/extensions/external_provider/
//! openai_auth/openai_account_models.py):
//! - live GET {base_url}/models with Bearer token (ureq, sync);
//! - parse: entries from models/data (list or dict), filter hide/hidden visibility,
//!   id from slug/id/name/model, sort by (priority, id), dedup, forward-compat expansion;
//! - fallback chain: live -> local JSON cache (model_ids or payload form) -> built-in defaults.

use std::fs;
use std::sync::Arc;

use ah_contracts::keys::MODEL_CATALOG;
use ah_contracts::model_catalog::{ModelCatalog, ModelCatalogError};
use ah_contracts::prelude::Effect;
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

/// Built-in default models (aligned with DEFAULT_OPENAI_ACCOUNT_MODELS).
pub const DEFAULT_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4-mini",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5-codex",
];

/// Forward-compat templates: synthetic model <- template models (any present appends).
pub const FORWARD_COMPAT_TEMPLATES: &[(&str, &[&str])] = &[
    (
        "gpt-5.5",
        &["gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex", "gpt-5-codex"],
    ),
    ("gpt-5.4-mini", &["gpt-5.3-codex", "gpt-5-codex"]),
    ("gpt-5.4", &["gpt-5.3-codex", "gpt-5-codex"]),
    ("gpt-5.3-codex-spark", &["gpt-5.3-codex"]),
];

/// Hidden visibility set (aligned with _HIDDEN_VISIBILITIES).
pub const HIDDEN_VISIBILITIES: &[&str] = &["hide", "hidden"];

fn is_hidden(visibility: Option<&str>) -> bool {
    visibility
        .map(|v| HIDDEN_VISIBILITIES.contains(&v.trim().to_lowercase().as_str()))
        .unwrap_or(false)
}

fn model_id(item: &Value) -> Option<String> {
    for key in ["slug", "id", "name", "model"] {
        if let Some(v) = item.get(key).and_then(Value::as_str) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn priority(item: &Value) -> i64 {
    match item.get("priority") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(10_000),
        _ => 10_000,
    }
}

/// Expand entries: models/data as list or dict; dict entries get a slug injected.
fn model_entries(payload: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for key in ["models", "data"] {
        match payload.get(key) {
            Some(Value::Array(items)) => {
                out = items.clone();
                break;
            }
            Some(Value::Object(map)) => {
                for (id, meta) in map {
                    let mut entry = meta.clone();
                    if entry.get("slug").is_none() {
                        entry["slug"] = json!(id);
                    }
                    out.push(entry);
                }
                break;
            }
            _ => {}
        }
    }
    out
}

/// Dedup order-preserving model id sequence.
fn model_ids_from_sequence(value: &Value) -> Vec<String> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        if let Some(s) = item.as_str() {
            let s = s.trim();
            if !s.is_empty() && !out.iter().any(|x: &String| x == s) {
                out.push(s.to_string());
            }
        }
    }
    out
}

/// Real model catalog implementation.
pub struct ModelCatalogImpl;

impl Seam for ModelCatalogImpl {}

impl ModelCatalog for ModelCatalogImpl {
    fn parse_model_ids(&self, payload: &Value) -> Vec<String> {
        let mut sortable: Vec<(i64, String)> = Vec::new();
        for item in model_entries(payload) {
            if !item.is_object() {
                continue;
            }
            if is_hidden(item.get("visibility").and_then(Value::as_str)) {
                continue;
            }
            let Some(id) = model_id(&item) else {
                continue;
            };
            sortable.push((priority(&item), id));
        }
        sortable.sort();
        let ordered: Vec<String> = sortable.into_iter().map(|(_, id)| id).collect();
        self.add_forward_compat(ordered)
    }

    fn add_forward_compat(&self, model_ids: Vec<String>) -> Vec<String> {
        let mut ordered: Vec<String> = Vec::new();
        for id in model_ids {
            if !ordered.contains(&id) {
                ordered.push(id);
            }
        }
        for (synthetic, templates) in FORWARD_COMPAT_TEMPLATES {
            if ordered.iter().any(|m| m == synthetic) {
                continue;
            }
            if templates.iter().any(|t| ordered.iter().any(|m| m == t)) {
                ordered.push((*synthetic).to_string());
            }
        }
        ordered
    }

    fn fetch_models(
        &self,
        access_token: &str,
        base_url: &str,
    ) -> Result<(Value, Vec<String>), ModelCatalogError> {
        if access_token.trim().is_empty() {
            return Err(ModelCatalogError::new(
                "OpenAI account model discovery requires an access token.",
            ));
        }
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(10))
            .build();
        let response = match agent
            .get(&url)
            .set("Authorization", &format!("Bearer {access_token}"))
            .set("Accept", "application/json")
            .call()
        {
            Ok(resp) => resp,
            Err(ureq::Error::Status(code, _)) => {
                return Err(ModelCatalogError::with_status(
                    format!("OpenAI account model discovery failed with status {code}."),
                    code,
                ));
            }
            Err(e) => {
                return Err(ModelCatalogError::new(format!(
                    "models request failed: {e}"
                )));
            }
        };
        let status = response.status();
        if status != 200 {
            return Err(ModelCatalogError::with_status(
                format!("OpenAI account model discovery failed with status {status}."),
                status,
            ));
        }
        let body = response
            .into_string()
            .map_err(|e| ModelCatalogError::new(format!("read models body failed: {e}")))?;
        let payload: Value = serde_json::from_str(&body).map_err(|_| {
            ModelCatalogError::new("OpenAI account model discovery returned invalid JSON.")
        })?;
        if !payload.is_object() {
            return Err(ModelCatalogError::new(
                "OpenAI account model discovery returned an unsupported payload.",
            ));
        }
        let model_ids = self.parse_model_ids(&payload);
        if model_ids.is_empty() {
            return Err(ModelCatalogError::new(
                "OpenAI account model discovery returned no visible models.",
            ));
        }
        Ok((payload, model_ids))
    }

    fn list_model_ids(
        &self,
        access_token: Option<&str>,
        base_url: &str,
        cache_path: &str,
        force_refresh: bool,
    ) -> Vec<String> {
        let _ = force_refresh;
        if let Some(token) = access_token.filter(|t| !t.trim().is_empty()) {
            match self.fetch_models(token, base_url) {
                Ok((payload, model_ids)) => {
                    if let Err(e) = self.write_cache(cache_path, &payload, &model_ids) {
                        eprintln!("[model-catalog] cache write failed: {e}");
                    }
                    return model_ids;
                }
                Err(e) => {
                    eprintln!("[model-catalog] live fetch failed: {e}");
                }
            }
        }
        let cached = self.read_cache(cache_path);
        if !cached.is_empty() {
            return cached;
        }
        self.add_forward_compat(DEFAULT_MODELS.iter().map(|s| s.to_string()).collect())
    }

    fn read_cache(&self, cache_path: &str) -> Vec<String> {
        let Ok(text) = fs::read_to_string(cache_path) else {
            return Vec::new();
        };
        let Ok(payload) = serde_json::from_str::<Value>(&text) else {
            return Vec::new();
        };
        if !payload.is_object() {
            return Vec::new();
        }
        let cached_ids = model_ids_from_sequence(payload.get("model_ids").unwrap_or(&Value::Null));
        if !cached_ids.is_empty() {
            return self.add_forward_compat(cached_ids);
        }
        if let Some(p) = payload.get("payload") {
            return self.parse_model_ids(p);
        }
        self.parse_model_ids(&payload)
    }

    fn write_cache(
        &self,
        cache_path: &str,
        payload: &Value,
        model_ids: &[String],
    ) -> Result<(), ModelCatalogError> {
        if model_ids.is_empty() {
            return Ok(());
        }
        if let Some(parent) = std::path::Path::new(cache_path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let data = json!({
            "provider": "openai-account",
            "model_ids": model_ids,
            "payload": payload,
        });
        let text = serde_json::to_string_pretty(&data)
            .map_err(|e| ModelCatalogError::new(format!("serialize cache failed: {e}")))?;
        fs::write(cache_path, text)
            .map_err(|e| ModelCatalogError::new(format!("write cache failed: {e}")))?;
        Ok(())
    }
}

/// model-catalog plugin: registers the `model-catalog` seam.
pub struct ModelCatalogPlugin;

impl Plugin for ModelCatalogPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-model-catalog"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![MODEL_CATALOG]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let catalog: Arc<dyn ModelCatalog> = Arc::new(ModelCatalogImpl);
        Ok(vec![ctx.register(MODEL_CATALOG, catalog)])
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use ah_contracts::keys::MODEL_CATALOG;
    use ah_hub::plugin::DynPlugin;
    use serde_json::json;

    fn temp_file(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ah-model-catalog-{tag}-{}", std::process::id()))
    }

    /// 本地 /models 服务器:返回给定 payload(或状态码)。
    fn start_models_server(payload: &'static str, status: u16) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind");
        let port = server.server_addr().to_ip().expect("ip").port();
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let response = if status == 200 {
                    tiny_http::Response::from_string(payload.to_string())
                } else {
                    tiny_http::Response::from_string(r#"{"error":{"message":"boom"}}"#)
                        .with_status_code(status)
                };
                let _ = request.respond(response);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn parse_orders_by_priority_filters_hidden_and_dedups() {
        let catalog = ModelCatalogImpl;
        let payload = json!({
            "models": [
                { "slug": "model-z", "priority": 1 },
                { "slug": "model-a", "priority": 2 },
                { "slug": "hidden-model", "visibility": "hidden" },
                { "slug": "model-z" }, // 重复 → 去重
            ],
        });
        let ids = catalog.parse_model_ids(&payload);
        assert_eq!(ids[0], "model-z", "priority 1 排最前");
        assert_eq!(ids[1], "model-a");
        assert!(!ids.contains(&"hidden-model".to_string()), "hidden 被过滤");
        assert_eq!(ids.iter().filter(|m| m.as_str() == "model-z").count(), 1);
    }

    #[test]
    fn parse_accepts_dict_models_and_data_fallback() {
        let catalog = ModelCatalogImpl;
        // models 为 dict:slug 注入。
        let dict_payload = json!({
            "models": {
                "alpha": { "priority": 5 },
                "beta": { "visibility": "hide" },
            },
        });
        let ids = catalog.parse_model_ids(&dict_payload);
        assert_eq!(ids[0], "alpha", "dict key 注入 slug");
        assert!(!ids.contains(&"beta".to_string()));
        // 无 models → data 回退。
        let data_payload = json!({ "data": [{ "id": "d1" }, { "id": "d2" }] });
        let ids2 = catalog.parse_model_ids(&data_payload);
        assert!(ids2.contains(&"d1".to_string()));
        assert!(ids2.contains(&"d2".to_string()));
        // 无 models/data → 空。
        assert!(catalog.parse_model_ids(&json!({ "other": [] })).is_empty());
    }

    #[test]
    fn forward_compat_appends_synthetic_when_template_present() {
        let catalog = ModelCatalogImpl;
        // gpt-5.4-mini 存在 → 追加 gpt-5.5(模板命中)。
        let ids = catalog.add_forward_compat(vec!["gpt-5.4-mini".to_string()]);
        assert!(
            ids.contains(&"gpt-5.5".to_string()),
            "合成模型追加: {ids:?}"
        );
        assert!(
            !ids.contains(&"gpt-5.4".to_string()),
            "gpt-5.4 不是 gpt-5.4-mini 的模板"
        );
        assert!(!ids.contains(&"gpt-5.3-codex".to_string()));
        // 已含合成模型 → 不重复。
        let ids2 =
            catalog.add_forward_compat(vec!["gpt-5.5".to_string(), "gpt-5.4-mini".to_string()]);
        assert_eq!(ids2.iter().filter(|m| m.as_str() == "gpt-5.5").count(), 1);
        // 无模板 → 不追加。
        let ids3 = catalog.add_forward_compat(vec!["unrelated".to_string()]);
        assert!(!ids3.contains(&"gpt-5.5".to_string()));
    }

    #[test]
    fn cache_write_read_roundtrip() {
        let catalog = ModelCatalogImpl;
        let path = temp_file("cache");
        let _ = fs::remove_file(&path);
        let payload = json!({ "models": [{ "id": "m1" }] });
        catalog
            .write_cache(
                path.to_str().unwrap(),
                &payload,
                &["m1".to_string(), "m2".to_string()],
            )
            .expect("write");
        let ids = catalog.read_cache(path.to_str().unwrap());
        assert_eq!(ids, vec!["m1".to_string(), "m2".to_string()]);
        // 无缓存文件 → 空。
        let _ = fs::remove_file(&path);
        assert!(catalog.read_cache(path.to_str().unwrap()).is_empty());
        // payload-only 缓存(无 model_ids 键)。
        let payload_only = json!({ "payload": { "models": [{ "id": "p1" }] } });
        fs::write(&path, payload_only.to_string()).unwrap();
        let ids2 = catalog.read_cache(path.to_str().unwrap());
        assert!(ids2.contains(&"p1".to_string()));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn fetch_models_live_and_error_paths() {
        let catalog = ModelCatalogImpl;
        // 无 token → 显式错误。
        let err = catalog.fetch_models("", "http://127.0.0.1:1").unwrap_err();
        assert!(err.message.contains("requires an access token"));
        // 500 → 带状态码错误。
        let base = start_models_server("{}", 500);
        let err = catalog.fetch_models("tok", &base).unwrap_err();
        assert_eq!(err.status_code, Some(500));
        // 200 → 解析真实负载。
        let payload =
            r#"{"models":[{"id":"gpt-5.4-mini"},{"id":"custom-1","visibility":"hidden"}]}"#;
        let base2 = start_models_server(payload, 200);
        let (_, ids) = catalog.fetch_models("tok", &base2).expect("fetch");
        assert!(
            ids.contains(&"gpt-5.4-mini".to_string()),
            "live 模型: {ids:?}"
        );
        assert!(!ids.contains(&"custom-1".to_string()));
        // gpt-5.4-mini 模板 → 合成模型追加。
        assert!(ids.contains(&"gpt-5.5".to_string()));
    }

    #[test]
    fn list_model_ids_fallback_chain() {
        let catalog = ModelCatalogImpl;
        let cache = temp_file("list");
        let _ = fs::remove_file(&cache);
        // 1) 无 token + 无缓存 → 内置默认 + 前向兼容。
        let ids =
            catalog.list_model_ids(None, "http://127.0.0.1:1", cache.to_str().unwrap(), false);
        assert!(ids.contains(&"gpt-5.5".to_string()));
        assert!(ids.contains(&"gpt-5-codex".to_string()));
        // 2) token + 实时成功 → live 模型 + 写缓存。
        let payload = r#"{"models":[{"id":"live-model"}]}"#;
        let base = start_models_server(payload, 200);
        let ids2 = catalog.list_model_ids(Some("tok"), &base, cache.to_str().unwrap(), false);
        assert!(ids2.contains(&"live-model".to_string()));
        assert!(cache.exists(), "实时成功后写缓存");
        // 3) token + 实时失败 → 缓存兜底。
        let ids3 = catalog.list_model_ids(
            Some("tok"),
            "http://127.0.0.1:1",
            cache.to_str().unwrap(),
            false,
        );
        assert!(
            ids3.contains(&"live-model".to_string()),
            "缓存兜底: {ids3:?}"
        );
        let _ = fs::remove_file(&cache);
    }

    #[test]
    fn plugin_registers_model_catalog() {
        let ctx = Context::new();
        let plugin: DynPlugin = Arc::new(ModelCatalogPlugin);
        let effects = ctx.mount(&plugin).expect("mount");
        let catalog = ctx
            .service::<dyn ModelCatalog>(&MODEL_CATALOG)
            .expect("catalog");
        assert!(catalog.parse_model_ids(&json!({})).is_empty());
        drop(effects);
        assert!(!ctx.has_service(&MODEL_CATALOG));
    }
}
