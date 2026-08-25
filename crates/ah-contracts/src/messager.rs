//! messager seam:团队消息传输抽象(对齐 agent_teams/messager/base.py + inprocess.py)。
//!
//! 确定性部分:
//! - `MessagerPeerConfig` / `MessagerTransportConfig`(含 broadcast_topic)/
//!   `SubscriptionHandle` 纯模型 + `create_messager` 后端分派(inprocess 支持,
//!   pyzmq 需外部依赖 → 显式报错);
//! - `InProcessMessager`:进程内 pub-sub + P2P 总线(subscribe/unsubscribe/
//!   publish/send/register·unregister_direct_message_handler),handler 直接
//!   调用,无序列化;与 Python `_Bus` 语义一致(topic → agent_id → handler,
//!   p2p agent_id → handler)。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::seam::Seam;

/// 消息处理器(对齐 MessagerHandler)。
pub type MessagerHandler = Arc<dyn Fn(Value) + Send + Sync>;

/// 静态 peer 元数据(对齐 MessagerPeerConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MessagerPeerConfig {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default)]
    pub addrs: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Map<String, Value>,
}

/// 消息传输配置(对齐 MessagerTransportConfig)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MessagerTransportConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default = "default_team_name")]
    pub team_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubsub_publish_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubsub_subscribe_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_publish_url: Option<String>,
    #[serde(default)]
    pub listen_addrs: Vec<String>,
    #[serde(default)]
    pub bootstrap_peers: Vec<MessagerPeerConfig>,
    #[serde(default)]
    pub known_peers: Vec<MessagerPeerConfig>,
    #[serde(default = "default_request_timeout")]
    pub request_timeout: f64,
    #[serde(default)]
    pub metadata: serde_json::Map<String, Value>,
}

fn default_backend() -> String {
    "inprocess".to_string()
}
fn default_team_name() -> String {
    "default".to_string()
}
fn default_request_timeout() -> f64 {
    10.0
}

impl Default for MessagerTransportConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            team_name: default_team_name(),
            node_id: None,
            direct_addr: None,
            pubsub_publish_addr: None,
            pubsub_subscribe_addr: None,
            external_publish_url: None,
            listen_addrs: vec![],
            bootstrap_peers: vec![],
            known_peers: vec![],
            request_timeout: default_request_timeout(),
            metadata: serde_json::Map::new(),
        }
    }
}

impl MessagerTransportConfig {
    /// 广播主题(对齐 `broadcast_topic`):`team:{team_name}:broadcast`。
    pub fn broadcast_topic(&self) -> String {
        format!("team:{}:broadcast", self.team_name)
    }
}

/// 订阅句柄(对齐 SubscriptionHandle)。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionHandle {
    pub subscription_id: String,
    pub topic: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub backend_metadata: serde_json::Map<String, Value>,
}

/// 消息传输抽象(对齐 Messager)。
pub trait Messager: Seam {
    fn start(&self);
    fn stop(&self);
    /// 向主题发布事件(带 sender_id 盖章)。
    fn publish(&self, topic_id: &str, message: Value);
    /// 订阅主题(handler 直接调用)。
    fn subscribe(&self, topic_id: &str, handler: MessagerHandler);
    fn unsubscribe(&self, topic_id: &str);
    /// 点对点发送。
    fn send(&self, agent_id: &str, message: Value);
    fn register_direct_message_handler(&self, handler: MessagerHandler);
    fn unregister_direct_message_handler(&self);
}

/// 创建消息传输(对齐 `create_messager`):inprocess → InProcessMessager;
/// 其他后端(pyzmq 等需外部依赖)显式报错。
pub fn create_messager(
    config: MessagerTransportConfig,
) -> Result<Arc<dyn Messager>, MessagerError> {
    match config.backend.as_str() {
        "inprocess" => Ok(Arc::new(InProcessMessager::new(config))),
        other => Err(MessagerError(format!(
            "Unsupported messager backend: {other}"
        ))),
    }
}

/// messager 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagerError(pub String);

impl core::fmt::Display for MessagerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MessagerError {}

/// 进程内消息总线(对齐 `_Bus` 的确定性数据结构)。
#[derive(Default)]
pub struct InProcessBus {
    /// topic → agent_id → handler。
    topic_subs: Mutex<HashMap<String, HashMap<String, MessagerHandler>>>,
    /// agent_id → handler(p2p)。
    p2p: Mutex<HashMap<String, MessagerHandler>>,
}

impl InProcessBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&self, agent_id: &str, topic: &str, handler: MessagerHandler) {
        self.topic_subs
            .lock()
            .unwrap()
            .entry(topic.to_string())
            .or_default()
            .insert(agent_id.to_string(), handler);
    }

    pub fn unsubscribe(&self, agent_id: &str, topic: &str) {
        let mut subs = self.topic_subs.lock().unwrap();
        if let Some(topic_map) = subs.get_mut(topic) {
            topic_map.remove(agent_id);
            if topic_map.is_empty() {
                subs.remove(topic);
            }
        }
    }

    /// 发布:逐 handler 调用;单个 handler 异常不影响其余(对齐 publish)。
    pub fn publish(&self, topic: &str, message: &Value) {
        let snapshot: Vec<MessagerHandler> = self
            .topic_subs
            .lock()
            .unwrap()
            .get(topic)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        for handler in snapshot {
            handler(message.clone());
        }
    }

    pub fn register_p2p(&self, agent_id: &str, handler: MessagerHandler) {
        self.p2p
            .lock()
            .unwrap()
            .insert(agent_id.to_string(), handler);
    }

    pub fn unregister_p2p(&self, agent_id: &str) {
        self.p2p.lock().unwrap().remove(agent_id);
    }

    pub fn send(&self, agent_id: &str, message: &Value) -> bool {
        let Some(handler) = self.p2p.lock().unwrap().get(agent_id).cloned() else {
            return false;
        };
        handler(message.clone());
        true
    }

    pub fn clear(&self) {
        self.topic_subs.lock().unwrap().clear();
        self.p2p.lock().unwrap().clear();
    }
}

/// 进程内消息传输(对齐 InProcessMessager)。
pub struct InProcessMessager {
    config: MessagerTransportConfig,
    bus: Arc<InProcessBus>,
    subscribed_topics: Mutex<Vec<String>>,
}

impl InProcessMessager {
    pub fn new(config: MessagerTransportConfig) -> Self {
        Self {
            config,
            bus: Arc::new(InProcessBus::new()),
            subscribed_topics: Mutex::new(Vec::new()),
        }
    }

    fn agent_id(&self) -> &str {
        self.config.node_id.as_deref().unwrap_or("")
    }
}

impl Seam for InProcessMessager {}

impl Messager for InProcessMessager {
    fn start(&self) {}
    fn stop(&self) {}

    fn publish(&self, topic_id: &str, message: Value) {
        // 对齐 Python:消息 sender_id 为空时盖章为 agent_id。
        let stamped = if let Some(obj) = message.as_object() {
            let mut message = message.clone();
            let sender = obj.get("sender_id").cloned().unwrap_or(Value::Null);
            let needs_stamp =
                sender.is_null() || sender.as_str().map(|s| s.is_empty()).unwrap_or(false);
            if needs_stamp && let Some(obj) = message.as_object_mut() {
                obj.insert(
                    "sender_id".to_string(),
                    Value::String(self.agent_id().to_string()),
                );
            }
            message
        } else {
            message
        };
        self.bus.publish(topic_id, &stamped);
    }

    fn subscribe(&self, topic_id: &str, handler: MessagerHandler) {
        self.bus.subscribe(self.agent_id(), topic_id, handler);
        self.subscribed_topics
            .lock()
            .unwrap()
            .push(topic_id.to_string());
    }

    fn unsubscribe(&self, topic_id: &str) {
        self.bus.unsubscribe(self.agent_id(), topic_id);
        self.subscribed_topics
            .lock()
            .unwrap()
            .retain(|t| t != topic_id);
    }

    fn send(&self, agent_id: &str, message: Value) {
        self.bus.send(agent_id, &message);
    }

    fn register_direct_message_handler(&self, handler: MessagerHandler) {
        self.bus.register_p2p(self.agent_id(), handler);
    }

    fn unregister_direct_message_handler(&self) {
        self.bus.unregister_p2p(self.agent_id());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn transport_config_defaults_and_broadcast_topic() {
        let cfg = MessagerTransportConfig::default();
        assert_eq!(cfg.backend, "inprocess");
        assert_eq!(cfg.team_name, "default");
        assert_eq!(cfg.request_timeout, 10.0);
        assert_eq!(cfg.broadcast_topic(), "team:default:broadcast");
        let cfg2 = MessagerTransportConfig {
            team_name: "alpha".to_string(),
            ..Default::default()
        };
        assert_eq!(cfg2.broadcast_topic(), "team:alpha:broadcast");
    }

    #[test]
    fn create_messager_dispatches_or_errors() {
        let cfg = MessagerTransportConfig::default();
        let _m = create_messager(cfg).expect("inprocess");
        // 未知后端显式报错。
        let bad = MessagerTransportConfig {
            backend: "pyzmq".to_string(),
            ..Default::default()
        };
        match create_messager(bad) {
            Err(err) => assert!(err.0.contains("Unsupported messager backend: pyzmq")),
            Ok(_) => panic!("pyzmq should be rejected"),
        }
    }

    #[test]
    fn inprocess_pubsub_and_p2p() {
        let cfg = MessagerTransportConfig {
            node_id: Some("leader".to_string()),
            team_name: "alpha".to_string(),
            ..Default::default()
        };
        let messager = InProcessMessager::new(cfg);
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        messager.subscribe(
            "team:alpha:broadcast",
            Arc::new(move |_msg| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        messager.publish("team:alpha:broadcast", json!({"type": "x"}));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // 点对点。
        let bus = Arc::new(InProcessBus::new());
        let handler_calls = Arc::new(AtomicUsize::new(0));
        let hc = handler_calls.clone();
        bus.register_p2p(
            "teammate",
            Arc::new(move |_msg| {
                hc.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(bus.send("teammate", &json!({"type": "y"})));
        assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
        assert!(!bus.send("ghost", &json!({})), "no handler → false");
        bus.unregister_p2p("teammate");
        assert!(!bus.send("teammate", &json!({})));
    }

    #[test]
    fn publish_stamps_sender_id() {
        let cfg = MessagerTransportConfig {
            node_id: Some("leader".to_string()),
            ..Default::default()
        };
        let messager = InProcessMessager::new(cfg);
        let captured = Arc::new(Mutex::new(Value::Null));
        let cap = captured.clone();
        messager.subscribe(
            "topic",
            Arc::new(move |msg| {
                *cap.lock().unwrap() = msg;
            }),
        );
        messager.publish("topic", json!({"type": "t", "sender_id": ""}));
        assert_eq!(captured.lock().unwrap()["sender_id"], "leader");
    }

    #[test]
    fn unsubscribe_removes_and_bus_clear() {
        let bus = Arc::new(InProcessBus::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        bus.subscribe(
            "a1",
            "topic",
            Arc::new(move |_msg| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        bus.publish("topic", &json!({}));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        bus.unsubscribe("a1", "topic");
        bus.publish("topic", &json!({}));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "unsubscribed");
        bus.clear();
    }
}
