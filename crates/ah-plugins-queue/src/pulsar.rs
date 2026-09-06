//! Pulsar REST proxy queue backend.
//!
//! This adapter targets the Pulsar REST proxy topic endpoints. The queue
//! contract's local sequence is assigned from the proxy response when a
//! numeric `seq` is returned; otherwise it is maintained by the client for
//! the current process. Durable broker acknowledgement remains the source of
//! delivery, while `consume` requires the proxy to return a full
//! `QueueMessage` JSON object.

use std::sync::atomic::{AtomicU64, Ordering};

use ah_contracts::keys::QUEUE;
use ah_contracts::prelude::Effect;
use ah_contracts::queue::{MessageQueue, QueueError, QueueMessage};
use ah_contracts::seam::Seam;
use ah_contracts::service::ServiceKey;
use ah_hub::context::Context;
use ah_hub::plugin::{Plugin, PluginError};
use serde_json::{Value, json};

/// Pulsar REST proxy implementation of `MessageQueue`.
pub struct PulsarRestQueue {
    base_url: String,
    tenant: String,
    namespace: String,
    subscription: String,
    token: Option<String>,
    next_seq: AtomicU64,
    agent: ureq::Agent,
}

impl PulsarRestQueue {
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        namespace: impl Into<String>,
        subscription: impl Into<String>,
        token: Option<String>,
    ) -> Result<Self, QueueError> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(QueueError("pulsar URL must use http or https".into()));
        }
        let tenant = tenant.into();
        let namespace = namespace.into();
        let subscription = subscription.into();
        if [tenant.as_str(), namespace.as_str(), subscription.as_str()]
            .iter()
            .any(|part| part.is_empty() || part.contains('/'))
        {
            return Err(QueueError(
                "pulsar tenant, namespace, and subscription must be path segments".into(),
            ));
        }
        Ok(Self {
            base_url,
            tenant,
            namespace,
            subscription,
            token,
            next_seq: AtomicU64::new(0),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(15))
                .build(),
        })
    }

    fn topic_path(&self, channel: &str) -> Result<String, QueueError> {
        if channel.is_empty() || channel.contains('/') {
            return Err(QueueError("pulsar channel must be one path segment".into()));
        }
        Ok(format!(
            "/topics/persistent/{}/{}/{}",
            self.tenant, self.namespace, channel
        ))
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        let request = self
            .agent
            .request(method, &format!("{}{path}", self.base_url));
        if let Some(token) = &self.token {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        }
    }

    fn response_json(
        &self,
        response: ureq::Response,
        operation: &str,
    ) -> Result<Value, QueueError> {
        response
            .into_json()
            .map_err(|error| QueueError(format!("pulsar {operation} JSON: {error}")))
    }
}

impl Seam for PulsarRestQueue {}

impl MessageQueue for PulsarRestQueue {
    fn publish(&self, channel: &str, payload: Value) -> Result<QueueMessage, QueueError> {
        let path = self.topic_path(channel)?;
        let body = self
            .request("POST", &path)
            .set("Content-Type", "application/json")
            .send_json(json!({"messages": [{"payload": payload}]}))
            .map_err(|error| QueueError(format!("pulsar publish {channel}: {error}")))?;
        let response = self.response_json(body, "publish")?;
        let seq = response
            .get("seq")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| self.next_seq.fetch_add(1, Ordering::SeqCst));
        Ok(QueueMessage {
            channel: channel.to_string(),
            seq,
            payload,
            ts_ms: now_ms(),
        })
    }

    fn consume(&self, channel: &str) -> Result<Option<QueueMessage>, QueueError> {
        let path = format!("{}/{}", self.topic_path(channel)?, self.subscription);
        let response = match self.request("GET", &path).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(204, _)) | Err(ureq::Error::Status(404, _)) => return Ok(None),
            Err(error) => return Err(QueueError(format!("pulsar consume {channel}: {error}"))),
        };
        let value = self.response_json(response, "consume")?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value)
            .map(Some)
            .map_err(|error| QueueError(format!("pulsar consume message: {error}")))
    }

    fn backlog(&self, channel: &str) -> Result<Vec<QueueMessage>, QueueError> {
        let path = format!(
            "{}/{}?backlog=true",
            self.topic_path(channel)?,
            self.subscription
        );
        let response = self
            .request("GET", &path)
            .call()
            .map_err(|error| QueueError(format!("pulsar backlog {channel}: {error}")))?;
        let value = self.response_json(response, "backlog")?;
        serde_json::from_value(value)
            .map_err(|error| QueueError(format!("pulsar backlog messages: {error}")))
    }

    fn channels(&self) -> Result<Vec<String>, QueueError> {
        let path = format!("/admin/v2/persistent/{}/{}", self.tenant, self.namespace);
        let response = self
            .request("GET", &path)
            .call()
            .map_err(|error| QueueError(format!("pulsar channels: {error}")))?;
        let value = self.response_json(response, "channels")?;
        let topics = value
            .as_array()
            .ok_or_else(|| QueueError("pulsar channels response must be an array".into()))?;
        topics
            .iter()
            .map(|topic| {
                let topic = topic
                    .as_str()
                    .ok_or_else(|| QueueError("pulsar channel entry must be a string".into()))?;
                topic
                    .rsplit('/')
                    .next()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| QueueError("pulsar topic has no channel name".into()))
            })
            .collect()
    }
}

/// Plugin wrapper for mounting a Pulsar REST queue in a profile.
pub struct PulsarRestQueuePlugin {
    base_url: String,
    tenant: String,
    namespace: String,
    subscription: String,
    token: Option<String>,
}

impl PulsarRestQueuePlugin {
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        namespace: impl Into<String>,
        subscription: impl Into<String>,
        token: Option<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            tenant: tenant.into(),
            namespace: namespace.into(),
            subscription: subscription.into(),
            token,
        }
    }
}

impl Plugin for PulsarRestQueuePlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-queue-pulsar"
    }

    fn provides(&self) -> Vec<ServiceKey> {
        vec![QUEUE]
    }

    fn inject(&self) -> Vec<ServiceKey> {
        vec![]
    }

    fn apply(&self, ctx: &Context) -> Result<Vec<Effect>, PluginError> {
        let queue = PulsarRestQueue::new(
            self.base_url.clone(),
            self.tenant.clone(),
            self.namespace.clone(),
            self.subscription.clone(),
            self.token.clone(),
        )
        .map_err(|error| PluginError::Apply {
            plugin: self.name(),
            message: error.0,
        })?;
        Ok(vec![ctx.register(QUEUE, std::sync::Arc::new(queue))])
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn publish_uses_persistent_topic_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 4096];
            let size = stream.read(&mut request).expect("request");
            let request = String::from_utf8_lossy(&request[..size]);
            assert!(request.starts_with("POST /topics/persistent/public/default/events"));
            let body = r#"{"seq":7}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("response");
        });
        let queue = PulsarRestQueue::new(
            format!("http://{address}"),
            "public",
            "default",
            "worker",
            Some("token".into()),
        )
        .expect("queue");
        let message = queue
            .publish("events", json!({"ok": true}))
            .expect("publish");
        assert_eq!(message.seq, 7);
        assert_eq!(message.payload["ok"], true);
        handle.join().expect("server");
    }
}
