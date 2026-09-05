//! Cross-process messager transport backed by libzmq.
//!
//! The transport matches the Python pyzmq topology: ROUTER/DEALER for direct
//! messages and PUB/SUB for topic broadcasts. Socket ownership stays in
//! dedicated threads because the public messager seam is synchronous.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ah_contracts::messager::{
    Messager, MessagerHandler, MessagerPeerConfig, MessagerTransportConfig,
};
use ah_contracts::seam::Seam;
use serde_json::Value;

const POLL_MS: i64 = 50;

struct RouterState {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

struct PubSubState {
    stop: Arc<AtomicBool>,
    commands: std::sync::mpsc::Sender<PubSubCommand>,
    join: Option<JoinHandle<()>>,
    broker_join: Option<JoinHandle<()>>,
}

enum PubSubCommand {
    Subscribe(String),
    Unsubscribe(String),
    Stop,
}

/// Synchronous cross-process messager using libzmq sockets.
pub struct PyzmqMessager {
    config: MessagerTransportConfig,
    context: Arc<zmq::Context>,
    peers: Mutex<HashMap<String, MessagerPeerConfig>>,
    p2p_handlers: Arc<Mutex<HashMap<String, MessagerHandler>>>,
    topic_handlers: Arc<Mutex<HashMap<String, MessagerHandler>>>,
    router: Mutex<Option<RouterState>>,
    pub_socket: Mutex<Option<zmq::Socket>>,
    pubsub: Mutex<Option<PubSubState>>,
    last_error: Mutex<Option<String>>,
}

impl PyzmqMessager {
    pub fn new(config: MessagerTransportConfig) -> Self {
        let peers = config
            .bootstrap_peers
            .iter()
            .chain(config.known_peers.iter())
            .map(|peer| (peer.agent_id.clone(), peer.clone()))
            .collect();
        Self {
            config,
            context: Arc::new(zmq::Context::new()),
            peers: Mutex::new(peers),
            p2p_handlers: Arc::new(Mutex::new(HashMap::new())),
            topic_handlers: Arc::new(Mutex::new(HashMap::new())),
            router: Mutex::new(None),
            pub_socket: Mutex::new(None),
            pubsub: Mutex::new(None),
            last_error: Mutex::new(None),
        }
    }

    pub fn register_peer(&self, peer: MessagerPeerConfig) {
        self.peers
            .lock()
            .unwrap()
            .insert(peer.agent_id.clone(), peer);
    }

    pub fn local_peer(&self) -> MessagerPeerConfig {
        MessagerPeerConfig {
            agent_id: self.config.node_id.clone().unwrap_or_default(),
            peer_id: None,
            addrs: self
                .config
                .direct_addr
                .as_ref()
                .map(|addr| vec![addr.clone()])
                .unwrap_or_default(),
            metadata: serde_json::Map::new(),
        }
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }

    fn set_error(&self, error: impl Into<String>) {
        *self.last_error.lock().unwrap() = Some(error.into());
    }

    fn timeout_ms(&self) -> i32 {
        (self.config.request_timeout.max(0.001) * 1000.0).round() as i32
    }

    fn start_inner(&self) -> Result<(), String> {
        if let Some(direct_addr) = self.config.direct_addr.as_deref() {
            let router = self
                .context
                .socket(zmq::ROUTER)
                .map_err(|e| format!("create ROUTER socket failed: {e}"))?;
            router
                .bind(direct_addr)
                .map_err(|e| format!("bind ROUTER at {direct_addr} failed: {e}"))?;
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let handlers = self.p2p_handlers.clone();
            let join = thread::Builder::new()
                .name("ah-messager-router".into())
                .spawn(move || router_loop(router, thread_stop, handlers))
                .map_err(|e| format!("spawn ROUTER thread failed: {e}"))?;
            *self.router.lock().unwrap() = Some(RouterState {
                stop,
                join: Some(join),
            });
        }

        if self.config.pubsub_publish_addr.is_some() || self.config.pubsub_subscribe_addr.is_some()
        {
            self.start_pubsub()?;
        }
        if self.router.lock().unwrap().is_none() && self.pubsub.lock().unwrap().is_none() {
            return Err("pyzmq messager requires direct_addr or pubsub addresses".to_string());
        }
        Ok(())
    }

    fn start_pubsub(&self) -> Result<(), String> {
        let publish_addr = self
            .config
            .pubsub_publish_addr
            .as_deref()
            .ok_or_else(|| "pyzmq messager requires pubsub_publish_addr".to_string())?;
        let subscribe_addr = self
            .config
            .pubsub_subscribe_addr
            .as_deref()
            .ok_or_else(|| "pyzmq messager requires pubsub_subscribe_addr".to_string())?;
        let bind_proxy = self
            .config
            .metadata
            .get("pubsub_bind")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stop = Arc::new(AtomicBool::new(false));
        let mut broker_join = None;
        if bind_proxy {
            let xsub = self
                .context
                .socket(zmq::XSUB)
                .map_err(|e| format!("create XSUB socket failed: {e}"))?;
            xsub.bind(publish_addr)
                .map_err(|e| format!("bind XSUB at {publish_addr} failed: {e}"))?;
            let xpub = self
                .context
                .socket(zmq::XPUB)
                .map_err(|e| format!("create XPUB socket failed: {e}"))?;
            xpub.bind(subscribe_addr)
                .map_err(|e| format!("bind XPUB at {subscribe_addr} failed: {e}"))?;
            let thread_stop = stop.clone();
            broker_join = Some(
                thread::Builder::new()
                    .name("ah-messager-proxy".into())
                    .spawn(move || proxy_loop(xsub, xpub, thread_stop))
                    .map_err(|e| format!("spawn PUB/SUB proxy failed: {e}"))?,
            );
        }
        let publisher = self
            .context
            .socket(zmq::PUB)
            .map_err(|e| format!("create PUB socket failed: {e}"))?;
        publisher
            .set_sndtimeo(self.timeout_ms())
            .map_err(|e| format!("set PUB timeout failed: {e}"))?;
        publisher
            .connect(publish_addr)
            .map_err(|e| format!("connect PUB to {publish_addr} failed: {e}"))?;
        *self.pub_socket.lock().unwrap() = Some(publisher);

        let subscriber = self
            .context
            .socket(zmq::SUB)
            .map_err(|e| format!("create SUB socket failed: {e}"))?;
        subscriber
            .set_rcvtimeo(POLL_MS as i32)
            .map_err(|e| format!("set SUB timeout failed: {e}"))?;
        subscriber
            .connect(subscribe_addr)
            .map_err(|e| format!("connect SUB to {subscribe_addr} failed: {e}"))?;
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let thread_stop = stop.clone();
        let handlers = self.topic_handlers.clone();
        let join = thread::Builder::new()
            .name("ah-messager-sub".into())
            .spawn(move || subscriber_loop(subscriber, commands_rx, thread_stop, handlers))
            .map_err(|e| format!("spawn SUB thread failed: {e}"))?;
        *self.pubsub.lock().unwrap() = Some(PubSubState {
            stop,
            commands: commands_tx,
            join: Some(join),
            broker_join,
        });
        // PUB/SUB has a slow-joiner window; let proxy and subscriptions settle.
        thread::sleep(Duration::from_millis(200));
        Ok(())
    }

    fn peer_addr(&self, agent_id: &str) -> Result<String, String> {
        self.peers
            .lock()
            .unwrap()
            .get(agent_id)
            .and_then(|peer| peer.addrs.first())
            .cloned()
            .ok_or_else(|| format!("unknown zmq route for recipient '{agent_id}'"))
    }

    fn send_inner(&self, agent_id: &str, message: Value) -> Result<(), String> {
        let address = self.peer_addr(agent_id)?;
        let dealer = self
            .context
            .socket(zmq::DEALER)
            .map_err(|e| format!("create DEALER socket failed: {e}"))?;
        if let Some(node_id) = &self.config.node_id {
            dealer
                .set_identity(node_id.as_bytes())
                .map_err(|e| format!("set DEALER identity failed: {e}"))?;
        }
        dealer
            .set_sndtimeo(self.timeout_ms())
            .and_then(|_| dealer.set_rcvtimeo(self.timeout_ms()))
            .map_err(|e| format!("set DEALER timeout failed: {e}"))?;
        dealer
            .connect(&address)
            .map_err(|e| format!("connect DEALER to {address} failed: {e}"))?;
        let mut payload = message;
        if let Some(object) = payload.as_object_mut() {
            object.insert("_recipient_id".into(), Value::String(agent_id.to_string()));
        }
        let bytes =
            serde_json::to_vec(&payload).map_err(|e| format!("serialize message failed: {e}"))?;
        dealer
            .send(bytes, 0)
            .map_err(|e| format!("send DEALER message failed: {e}"))?;
        dealer
            .recv_bytes(0)
            .map_err(|e| format!("receive DEALER acknowledgement failed: {e}"))?;
        Ok(())
    }
}

impl Seam for PyzmqMessager {}

impl Messager for PyzmqMessager {
    fn start(&self) {
        if self.router.lock().unwrap().is_some() || self.pubsub.lock().unwrap().is_some() {
            return;
        }
        if let Err(error) = self.start_inner() {
            self.set_error(error);
        }
    }

    fn stop(&self) {
        if let Some(mut state) = self.pubsub.lock().unwrap().take() {
            state.stop.store(true, Ordering::SeqCst);
            let _ = state.commands.send(PubSubCommand::Stop);
            if let Some(join) = state.join.take() {
                let _ = join.join();
            }
            if let Some(join) = state.broker_join.take() {
                let _ = join.join();
            }
        }
        self.pub_socket.lock().unwrap().take();
        if let Some(mut state) = self.router.lock().unwrap().take() {
            state.stop.store(true, Ordering::SeqCst);
            if let Some(join) = state.join.take() {
                let _ = join.join();
            }
        }
    }

    fn publish(&self, topic_id: &str, message: Value) {
        self.start();
        let message = stamp_sender(message, self.config.node_id.as_deref().unwrap_or(""));
        let payload = match serde_json::to_vec(&message) {
            Ok(payload) => payload,
            Err(error) => {
                self.set_error(format!("serialize PUB message failed: {error}"));
                return;
            }
        };
        let mut guard = self.pub_socket.lock().unwrap();
        if let Some(socket) = guard.as_mut()
            && let Err(error) = socket
                .send(topic_id, zmq::SNDMORE)
                .and_then(|_| socket.send(payload, 0))
        {
            self.set_error(format!("publish topic failed: {error}"));
        }
    }

    fn subscribe(&self, topic_id: &str, handler: MessagerHandler) {
        self.start();
        self.topic_handlers
            .lock()
            .unwrap()
            .insert(topic_id.to_string(), handler);
        if let Some(state) = self.pubsub.lock().unwrap().as_ref() {
            let _ = state
                .commands
                .send(PubSubCommand::Subscribe(topic_id.to_string()));
        }
    }

    fn unsubscribe(&self, topic_id: &str) {
        self.topic_handlers.lock().unwrap().remove(topic_id);
        if let Some(state) = self.pubsub.lock().unwrap().as_ref() {
            let _ = state
                .commands
                .send(PubSubCommand::Unsubscribe(topic_id.to_string()));
        }
    }

    fn send(&self, agent_id: &str, message: Value) {
        self.start();
        if let Err(error) = self.send_inner(agent_id, message) {
            self.set_error(error);
        }
    }

    fn register_direct_message_handler(&self, handler: MessagerHandler) {
        self.start();
        let agent_id = self.config.node_id.clone().unwrap_or_default();
        self.p2p_handlers.lock().unwrap().insert(agent_id, handler);
    }

    fn unregister_direct_message_handler(&self) {
        let agent_id = self.config.node_id.clone().unwrap_or_default();
        self.p2p_handlers.lock().unwrap().remove(&agent_id);
    }
}

impl Drop for PyzmqMessager {
    fn drop(&mut self) {
        self.stop();
    }
}

fn stamp_sender(mut message: Value, sender: &str) -> Value {
    if let Some(object) = message.as_object_mut() {
        let missing = object
            .get("sender_id")
            .map(|value| value.is_null() || value.as_str() == Some(""))
            .unwrap_or(true);
        if missing {
            object.insert("sender_id".into(), Value::String(sender.to_string()));
        }
    }
    message
}

fn router_loop(
    router: zmq::Socket,
    stop: Arc<AtomicBool>,
    handlers: Arc<Mutex<HashMap<String, MessagerHandler>>>,
) {
    while !stop.load(Ordering::SeqCst) {
        let mut items = [router.as_poll_item(zmq::POLLIN)];
        if zmq::poll(&mut items, POLL_MS).is_err() || !items[0].is_readable() {
            continue;
        }
        let Ok(frames) = router.recv_multipart(0) else {
            continue;
        };
        if frames.len() < 2 {
            continue;
        }
        let identity = &frames[0];
        let payload = &frames[frames.len() - 1];
        if let Ok(mut message) = serde_json::from_slice::<Value>(payload) {
            let recipient = message
                .get("_recipient_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(object) = message.as_object_mut() {
                object.remove("_recipient_id");
            }
            if let Some(handler) = handlers.lock().unwrap().get(&recipient).cloned() {
                handler(message);
            }
        }
        let _ = router.send_multipart([identity.as_slice(), b"ok"], 0);
    }
}

fn proxy_loop(xsub: zmq::Socket, xpub: zmq::Socket, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let mut items = [
            xsub.as_poll_item(zmq::POLLIN),
            xpub.as_poll_item(zmq::POLLIN),
        ];
        if zmq::poll(&mut items, POLL_MS).is_err() {
            continue;
        }
        if items[0].is_readable()
            && let Ok(frames) = xsub.recv_multipart(0)
        {
            let _ = xpub.send_multipart(frames, 0);
        }
        if items[1].is_readable()
            && let Ok(frames) = xpub.recv_multipart(0)
        {
            let _ = xsub.send_multipart(frames, 0);
        }
    }
}

fn subscriber_loop(
    subscriber: zmq::Socket,
    commands: std::sync::mpsc::Receiver<PubSubCommand>,
    stop: Arc<AtomicBool>,
    handlers: Arc<Mutex<HashMap<String, MessagerHandler>>>,
) {
    while !stop.load(Ordering::SeqCst) {
        while let Ok(command) = commands.try_recv() {
            match command {
                PubSubCommand::Subscribe(topic) => {
                    let _ = subscriber.set_subscribe(topic.as_bytes());
                }
                PubSubCommand::Unsubscribe(topic) => {
                    let _ = subscriber.set_unsubscribe(topic.as_bytes());
                }
                PubSubCommand::Stop => return,
            }
        }
        let Ok(frames) = subscriber.recv_multipart(0) else {
            continue;
        };
        if frames.len() < 2 {
            continue;
        }
        let topic = String::from_utf8_lossy(&frames[0]).into_owned();
        let Ok(message) = serde_json::from_slice::<Value>(&frames[1]) else {
            continue;
        };
        if let Some(handler) = handlers.lock().unwrap().get(&topic).cloned() {
            handler(message);
        }
    }
}

/// Dispatch a transport config to the in-process or cross-process backend.
pub fn create_messager(
    config: MessagerTransportConfig,
) -> Result<Arc<dyn Messager>, ah_contracts::messager::MessagerError> {
    match config.backend.as_str() {
        "inprocess" => Ok(ah_contracts::messager::create_messager(config)?),
        "pyzmq" => Ok(Arc::new(PyzmqMessager::new(config))),
        other => Err(ah_contracts::messager::MessagerError(format!(
            "Unsupported messager backend: {other}"
        ))),
    }
}

/// Plugin exposing the configured messager transport through the `messager` seam.
pub struct MessagerPlugin {
    config: MessagerTransportConfig,
}

impl MessagerPlugin {
    pub fn new(config: MessagerTransportConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        let mut config = MessagerTransportConfig {
            backend: std::env::var("AH_MESSAGER_BACKEND").unwrap_or_else(|_| "inprocess".into()),
            node_id: std::env::var("AH_MESSAGER_NODE_ID").ok(),
            direct_addr: std::env::var("AH_MESSAGER_DIRECT_ADDR").ok(),
            pubsub_publish_addr: std::env::var("AH_MESSAGER_PUBSUB_PUBLISH_ADDR").ok(),
            pubsub_subscribe_addr: std::env::var("AH_MESSAGER_PUBSUB_SUBSCRIBE_ADDR").ok(),
            ..Default::default()
        };
        if let Ok(value) = std::env::var("AH_MESSAGER_REQUEST_TIMEOUT")
            && let Ok(timeout) = value.parse()
        {
            config.request_timeout = timeout;
        }
        if std::env::var("AH_MESSAGER_PUBSUB_BIND").as_deref() == Ok("1") {
            config
                .metadata
                .insert("pubsub_bind".into(), Value::Bool(true));
        }
        Self::new(config)
    }
}

impl Default for MessagerPlugin {
    fn default() -> Self {
        Self::new(MessagerTransportConfig::default())
    }
}

impl ah_hub::plugin::Plugin for MessagerPlugin {
    fn name(&self) -> &'static str {
        "ah-plugins-messager"
    }

    fn provides(&self) -> Vec<ah_contracts::service::ServiceKey> {
        vec![ah_contracts::keys::MESSAGER]
    }

    fn inject(&self) -> Vec<ah_contracts::service::ServiceKey> {
        vec![]
    }

    fn apply(
        &self,
        ctx: &ah_hub::context::Context,
    ) -> Result<Vec<ah_contracts::prelude::Effect>, ah_hub::plugin::PluginError> {
        let messager = create_messager(self.config.clone()).map_err(|error| {
            ah_hub::plugin::PluginError::Apply {
                plugin: self.name(),
                message: error.0,
            }
        })?;
        Ok(vec![ctx.register(ah_contracts::keys::MESSAGER, messager)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    fn port(seed: u16) -> u16 {
        30_000 + (std::process::id() as u16 % 1_000) + seed
    }

    #[test]
    fn pyzmq_p2p_routes_message_and_acknowledges() {
        let a_addr = format!("tcp://127.0.0.1:{}", port(1));
        let b_addr = format!("tcp://127.0.0.1:{}", port(2));
        let a = PyzmqMessager::new(MessagerTransportConfig {
            backend: "pyzmq".into(),
            node_id: Some("a".into()),
            direct_addr: Some(a_addr.clone()),
            known_peers: vec![MessagerPeerConfig {
                agent_id: "b".into(),
                peer_id: None,
                addrs: vec![b_addr.clone()],
                metadata: serde_json::Map::new(),
            }],
            request_timeout: 2.0,
            ..Default::default()
        });
        let b = PyzmqMessager::new(MessagerTransportConfig {
            backend: "pyzmq".into(),
            node_id: Some("b".into()),
            direct_addr: Some(b_addr),
            request_timeout: 2.0,
            ..Default::default()
        });
        let received = Arc::new(AtomicUsize::new(0));
        let counter = received.clone();
        b.register_direct_message_handler(Arc::new(move |message| {
            assert_eq!(message["body"], "hello");
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        thread::sleep(Duration::from_millis(100));
        a.register_peer(b.local_peer());
        a.send("b", json!({"body":"hello"}));
        assert_eq!(a.last_error(), None);
        assert_eq!(received.load(Ordering::SeqCst), 1);
        a.stop();
        b.stop();
    }

    #[test]
    fn pyzmq_p2p_cross_process_delivery() {
        let child_mode = std::env::var_os("AH_P2P_CHILD").is_some();
        let child_addr =
            std::env::var("AH_P2P_ADDR").unwrap_or_else(|_| format!("tcp://127.0.0.1:{}", port(7)));
        let marker = std::env::var("AH_P2P_MARKER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("ah-pyzmq-marker-{}", std::process::id()))
            });
        let _ = std::fs::remove_file(&marker);
        if child_mode {
            let receiver = PyzmqMessager::new(MessagerTransportConfig {
                backend: "pyzmq".into(),
                node_id: Some("child".into()),
                direct_addr: Some(child_addr),
                request_timeout: 2.0,
                ..Default::default()
            });
            let marker_for_handler = marker.clone();
            receiver.register_direct_message_handler(Arc::new(move |message| {
                std::fs::write(&marker_for_handler, message.to_string()).expect("write marker");
            }));
            for _ in 0..100 {
                if marker.exists() {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
            panic!("cross-process message was not received");
        }

        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "tests::pyzmq_p2p_cross_process_delivery",
                "--nocapture",
            ])
            .env("AH_P2P_CHILD", "1")
            .env("AH_P2P_MARKER", &marker)
            .env("AH_P2P_ADDR", &child_addr)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn child test process");
        let parent = PyzmqMessager::new(MessagerTransportConfig {
            backend: "pyzmq".into(),
            node_id: Some("parent".into()),
            direct_addr: Some(format!("tcp://127.0.0.1:{}", port(8))),
            known_peers: vec![MessagerPeerConfig {
                agent_id: "child".into(),
                peer_id: None,
                addrs: vec![child_addr],
                metadata: serde_json::Map::new(),
            }],
            request_timeout: 2.0,
            ..Default::default()
        });
        thread::sleep(Duration::from_millis(250));
        parent.send("child", json!({"body":"cross-process"}));
        assert_eq!(parent.last_error(), None);
        let status = child.wait().expect("wait child");
        assert!(status.success(), "child process failed: {status}");
        let payload = std::fs::read_to_string(&marker).expect("read marker");
        assert_eq!(
            serde_json::from_str::<Value>(&payload).unwrap()["body"],
            "cross-process"
        );
        parent.stop();
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn pyzmq_pubsub_delivers_and_unsubscribes() {
        let pub_addr = format!("tcp://127.0.0.1:{}", port(3));
        let sub_addr = format!("tcp://127.0.0.1:{}", port(4));
        let mut broker_meta = serde_json::Map::new();
        broker_meta.insert("pubsub_bind".into(), Value::Bool(true));
        let publisher = PyzmqMessager::new(MessagerTransportConfig {
            backend: "pyzmq".into(),
            node_id: Some("publisher".into()),
            direct_addr: Some(format!("tcp://127.0.0.1:{}", port(5))),
            pubsub_publish_addr: Some(pub_addr.clone()),
            pubsub_subscribe_addr: Some(sub_addr.clone()),
            metadata: broker_meta,
            ..Default::default()
        });
        let subscriber = PyzmqMessager::new(MessagerTransportConfig {
            backend: "pyzmq".into(),
            node_id: Some("subscriber".into()),
            direct_addr: Some(format!("tcp://127.0.0.1:{}", port(6))),
            pubsub_publish_addr: Some(pub_addr),
            pubsub_subscribe_addr: Some(sub_addr),
            ..Default::default()
        });
        let received = Arc::new(AtomicUsize::new(0));
        let counter = received.clone();
        subscriber.subscribe(
            "team:test:broadcast",
            Arc::new(move |message| {
                assert_eq!(message["body"], "one");
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        thread::sleep(Duration::from_millis(250));
        publisher.publish("team:test:broadcast", json!({"body":"one"}));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(received.load(Ordering::SeqCst), 1);
        subscriber.unsubscribe("team:test:broadcast");
        publisher.publish("team:test:broadcast", json!({"body":"two"}));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(received.load(Ordering::SeqCst), 1);
        publisher.stop();
        subscriber.stop();
    }
}
